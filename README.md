拓展任务三：Linux SocketCAN 支持，以及Linux系统下USB协议的调整

一、 兼容协议说明
所选协议与驱动：gs_usb（Linux 内核标准 CAN over USB 协议，内置 gs_usb 驱动）。

上游来源：Linux 内核源码 drivers/net/can/usb/gs_usb.c 及经典开源固件 candleLight_fw。

$\color{red}{\text{关于candleLight_fw的理解}}$
一个标准的 USB-CAN 分析仪固件（与拓展1/2里用到的DMTool分析仪起到一样的作用）
将 MCU CAN 总线的数据转化为主机能明白的协议。
市面上买到的分析仪要求遵守厂家规定的私有协议并且使用对应的上位机软件来解析（DMTool）
但是使用candleLight_fw固件，遵循的是Linux内核中写好的、公开的标准协议，方便省钱
 （不仅不用购买还能直接在linux终端通过命令操控查看收发）

已实现的请求（在 src/main.rs 的 control_out / control_in 中处理）：

GS_USB_BREQ_BT_CONST (4)：主机查询设备位时序能力参数（返回 80MHz 内核时钟范围）。

GS_USB_BREQ_BITTIMING (1)：主机下发位时序参数（存入全局原子变量）。

GS_USB_BREQ_MODE (2)：处理 GS_CAN_MODE_START（启动）与 GS_CAN_MODE_RESET（复位）指令。

GS_USB_BREQ_DEVICE_CONFIG (5) 与 GS_USB_BREQ_HOST_FORMAT (0)：基础设备与格式配置。

$\color{red}{\text{关于“请求”的理解}}$
在 USB 通信中，数据传输分为两种：一种是传数据（e.g. CAN 帧通过 Bulk 端点发送），另一种是发指令/问问题。后者就是通过控制端点（EP0）发送的“请求”。请求一般在终端敲下加载驱动命令后开始，起到配置设备的作用。（设备不响应的话，系统驱动会认为这是个坏设备）

GS_USB_BREQ_BT_CONST：在加载驱动的时候。Linux 需要知道你的 STM32 的 FDCAN 内核时钟是多少（比如 80MHz），以及你支持的最大最小分频系数（BRP）、时间段长度（TSEG1/2）等。拿到这些参数，Linux 才能计算出 1Mbps、500kbps 等波特率对应的寄存器值。

GS_USB_BREQ_BITTIMING：在终端敲下 ip link set can0 up type can bitrate 1000000 的时候。Linux 根据上一步得到的参数，算好了 1Mbps 对应的具体参数（brp=1, prop_seg, phase_seg1/2 等），把这一包参数塞给设备，意思是：“按这个参数去配置你的 FDCAN 寄存器”。

GS_USB_BREQ_MODE：敲下 ip link set can0 up（启动）或者 ip link set can0 down（停止）的时候。开关作用。
             GS_CAN_MODE_START：告诉设备打开 CAN 控制器。

             GS_CAN_MODE_RESET：告诉设备关闭网卡了，请把 CAN 控制器停掉。

GS_USB_BREQ_DEVICE_CONFIG（设备->主机）、GS_USB_BREQ_HOST_FORMAT（主机->设备）：
            前者：支持几个CAN通道，固件版本是多少？此作业中只有can0
            后者：主机告诉设备，发出的结构体是大端还是小端



二， 数据路径与帧转换
本实现替换了原有的 CMSIS-DAP 与 CDC 串口功能，将 USB 枚举为单 Vendor 接口（VID: 0x1d50, PID: 0x606f），配置 EP1 IN（设备向主机发送）和 EP2 OUT（主机向设备发送）两个批量端点。

主机到 CAN（发送路径）：

usb_rx_task 从 EP2 OUT 读取原始字节流，按 #[repr(C)] 映射为 GsHostFrame（20字节结构体）。
提取 can_id、can_dlc 和 data[8]，过滤扩展帧。（candleLight_fw的功劳）
放入 CAN_TX_CHANNEL 有界队列。
can_tx_task 取出并构造 FdFrame 写入 FDCAN TX 邮箱。发送成功后，将原帧放回 ECHO_CHANNEL 触发 USB 回显。

$\color{red}{\text{关于收发时GsHostFrame结构体形式}}$
Linux 和 STM32 之间必须约定一个“打包格式”，才能知道这一串字节里，哪几个代表 ID，哪几个代表数据。
在拓展1/2中，用的是私有格式（比如文本字符串 CAN TX 123 55AA）。这就像你和你朋友私下写信，随便约定格式。

但在任务3中，面对的是 Linux 内核。Linux 不会去解析我自己的中文或英文字符串，它只认 gs_usb 官方规定的二进制格式——也就是 GsHostFrame。相当于一个接收发送标准。

$\color{red}{\text{为什么要“回显”？}}$
CAN 总线发送是不保证绝对成功的。 比如总线上如果有另一个设备同时发数据，就会发生“总线仲裁冲突”，这时候数据发送就失败了。

当让 Linux 发一帧数据到 can0 时，Linux 会给发一个 GsHostFrame结构体，并在 echo_id 里填一个唯一的流水号（比如 0x01）。

STM32 把这帧数据通过 FDCAN 扔到总线上。如果硬件发送成功，代码必须把刚才那个 GsHostFrame 原封不动（保留流水号 0x01）再通过 USB 传回给 Linux。

Linux 收到回传后，看到流水号是 0x01，就会在系统里标记：“哦！刚才那帧数据真的成功发到物理总线上了！” 然后它才会把这一帧显示在 candump 里。

如果没有回显，Linux 不知道数据到底发成功了没，干等超时。所以，回显就是 gs_usb 协议里的“发送成功确认（ACK）”。


CAN 到主机（接收路径）：

can_rx_task 从 FDCAN RX 中断读取数据。
解析出标准 ID 和载荷，填入 GsHostFrame（echo_id 设为 0xFFFFFFFF）。
放入 CAN_RX_CHANNEL 有界队列。
usb_tx_task 从队列取出，序列化为字节流通过 EP1 IN 发往 Linux。


三、 并发与缓冲处理
任务组织：基于 Embassy 异步运行时，拆分为 usb_rx_task、usb_tx_task、can_rx_task、can_tx_task，通过 embassy_futures::select 和 join 组合并发执行，互不阻塞。

队列与溢出处理：使用容量为 4 或 8 的 Channel 作为有界缓冲。所有队列写入均使用 try_send（非阻塞）。若队列已满（例如 USB 主机读取缓慢或 CAN 总线拥塞），直接丢弃新帧/旧帧，避免任务挂起。

发送回显：echo_id 机制完整保留，主机发送的帧在 CAN 物理层发送成功后，原封不动通过 EP1 IN 回传，供 Linux 驱动确认发送完成。


四、 关键配置与预期行为

Classical CAN 支持：本项目按要求实现基础范围的 Classical CAN 标准数据帧，载荷限制为 0-8 字节（GsHostFrame.data 为 [u8; 8]）。

FDCAN 底层配置：保留了拓展 2 的 FDCAN 初始化配置（1 Mbit/s 仲裁段，5 Mbit/s 数据段，启用 TDC）。可供后续启用CAN FD使用


五、 Linux 主机使用方法与验证

在 Linux 主机上无需安装额外驱动，内核自带 gs_usb 模块。设备插入后执行：

# 1. 加载驱动并查看接口
sudo modprobe gs_usb
ip -brief link show type can
# 预期输出: can0  DOWN  <NOARP,ECHO>

# 2. 启动 CAN 接口 (Classical CAN 1Mbit/s)
sudo ip link set can0 up type can bitrate 1000000

# 3. 接收测试 (另开终端)
candump can0

# 4. 发送测试
cansend can0 123#1122334455667788

预期行为：执行 cansend 后，由于 echo_id 回显机制，candump 窗口会立刻收到自己发送的这帧数据 can0 123 [8] 11 22 33 44 55 66 77 88。若外部总线上有其它节点发送数据，candump 也能同步接收显示。

$\color{red}{\text{与拓展1、拓展2 CAN测试的不同？}}$
首先分析仪不同及其使用的协议不同，前面提到过（私有、共有）
硬件连接、收发操作查看方式会不同。

硬件连接：1、虚拟机（以VM为例）

            插上板子，Windows 会尝试去认它（可能会弹窗提示“无法识别的设备”或要求安装驱动）。必须先阻止 Windows 认领它
            在虚拟机菜单栏找 虚拟机 (VM) -> 可移动设备 (Removable Devices)。
            找到 USB 设备（可能叫 Unknown Device，或者如果 VID/PID 识别了会叫 1d50:606f）。
            点击 连接 (断开与主机的连接)。
            Linux 会立刻加载 gs_usb 驱动。

         2、如果能接服务器
            把板子的 USB 插到服务器，接好 CAN 线。
            用 SSH 连进服务器。
            终端敲 dmesg -w，观察终端输出。
            当看到类似下面的日志：

            [  123.456789] usb 1-1: new full-speed USB device number 5 using xhci_hcd
            [  123.567890] gs_usb 1-1:1.0: Configuring for 1 channels...
            [  123.678901] gs_usb 1-1:1.0: registered as can0

            说明 Linux 已经成功通过 USB 识别到了你的板子。
            敲 ip link，应该就能看到一个名为 can0 的网络接口。

收发查看：拓展1/2在DMTool软件中通过切换窗口查看；拓展3直接在linux终端中通过命令查看。
