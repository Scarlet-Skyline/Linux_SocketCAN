#![no_std]
#![no_main]

mod board;
mod gs_usb;
mod wiring;

use board::Usb;
use embassy_executor::Spawner;
use embassy_futures::{
    join::{join, join4},
    select::{Either, select},
};
//use embassy_stm32::usart::BufferedUart;
use embassy_stm32::can::{CanRx, CanTx, frame::FdFrame};

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

//use embassy_time::{Duration, Ticker};
use embassy_usb::{
    Builder, Handler,
    control::{InResponse, OutResponse, Request, RequestType},
    driver::{Driver, Endpoint, EndpointIn, EndpointOut},
};

use embedded_can::{Frame, Id};
use panic_halt as _;
use static_cell::StaticCell;

use gs_usb::{
    CAN_STARTED, DEVICE_CONFIG, GS_CAN_MODE_RESET, GS_CAN_MODE_START, GS_SFF_MASK,
    GS_USB_BREQ_BITTIMING, GS_USB_BREQ_BT_CONST, GS_USB_BREQ_DEVICE_CONFIG,
    GS_USB_BREQ_HOST_FORMAT, GS_USB_BREQ_MODE, GsDeviceBittiming, GsDeviceBtConst, GsDeviceMode,
    GsHostFrame, NOMINAL_BRP, NOMINAL_SJW, NOMINAL_TSEG1, NOMINAL_TSEG2, bt_const,
    parse_host_frame, write_host_frame,
};

const PACKET: usize = 64;

/// CAN 接收 → 主机发送：容量 8 帧，满时丢弃最旧帧
static CAN_RX_CHANNEL: Channel<CriticalSectionRawMutex, GsHostFrame, 8> = Channel::new();

/// 主机请求 → CAN 发送：容量 4 帧，满时回 ERR BUSY
static CAN_TX_CHANNEL: Channel<CriticalSectionRawMutex, GsHostFrame, 4> = Channel::new();

/// TX echo → USB IN. Bounded 4.
static ECHO_CHANNEL: Channel<CriticalSectionRawMutex, GsHostFrame, 4> = Channel::new();

struct GsUsbHandler;

impl Handler for GsUsbHandler {
    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        if req.request_type != RequestType::Vendor {
            return None;
        }
        match req.request {
            GS_USB_BREQ_HOST_FORMAT => Some(OutResponse::Accepted),
            GS_USB_BREQ_BITTIMING => {
                if data.len() >= core::mem::size_of::<GsDeviceBittiming>() {
                    let bt = unsafe {
                        core::ptr::read_unaligned(data.as_ptr() as *const GsDeviceBittiming)
                    };
                    NOMINAL_BRP.store(bt.brp, core::sync::atomic::Ordering::Relaxed);
                    NOMINAL_TSEG1.store(
                        bt.prop_seg + bt.phase_seg1,
                        core::sync::atomic::Ordering::Relaxed,
                    );
                    NOMINAL_TSEG2.store(bt.phase_seg2, core::sync::atomic::Ordering::Relaxed);
                    NOMINAL_SJW.store(bt.sjw, core::sync::atomic::Ordering::Relaxed);
                }
                Some(OutResponse::Accepted)
            }
            GS_USB_BREQ_MODE => {
                if data.len() >= core::mem::size_of::<GsDeviceMode>() {
                    let m = unsafe { &*(data.as_ptr() as *const GsDeviceMode) };
                    if m.mode == GS_CAN_MODE_START {
                        CAN_STARTED.store(true, core::sync::atomic::Ordering::Release);
                    } else if m.mode == GS_CAN_MODE_RESET {
                        CAN_STARTED.store(false, core::sync::atomic::Ordering::Release);
                    }
                }
                Some(OutResponse::Accepted)
            }
            _ => Some(OutResponse::Accepted),
        }
    }

    fn control_in<'a>(&'a mut self, req: Request, data: &'a mut [u8]) -> Option<InResponse<'a>> {
        if req.request_type != RequestType::Vendor {
            return None;
        }
        match req.request {
            GS_USB_BREQ_BT_CONST => {
                let c = bt_const();
                let n = core::mem::size_of::<GsDeviceBtConst>().min(data.len());
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        &c as *const GsDeviceBtConst as *const u8,
                        data.as_mut_ptr(),
                        n,
                    );
                }
                //let slice: &'a [u8] = data;
                Some(InResponse::Accepted(&data[..n]))
            }
            GS_USB_BREQ_DEVICE_CONFIG => {
                let n = core::mem::size_of::<gs_usb::GsDeviceConfig>().min(data.len());
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        &DEVICE_CONFIG as *const gs_usb::GsDeviceConfig as *const u8,
                        data.as_mut_ptr(),
                        n,
                    );
                }
                //let slice: &'a [u8] = data;
                Some(InResponse::Accepted(&data[..n]))
            }
            _ => None,
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let (driver, ports) = wiring::configure(board::init());
    // Classroom-only test VID/PID, not allocated product identifiers.
    let mut config = embassy_usb::Config::new(0x1d50, 0x606f);
    config.manufacturer = Some("HerKules");
    config.product = Some("DM-MC02 gs_usb Demo");
    config.serial_number = Some("0001");
    config.max_packet_size_0 = PACKET as u8;

    static DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS: StaticCell<[u8; 128]> = StaticCell::new();
    static CONTROL: StaticCell<[u8; 64]> = StaticCell::new();
    static HANDLER: StaticCell<GsUsbHandler> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        DESC.init([0; 256]),
        BOS.init([0; 128]),
        MSOS.init([0; 256]),
        CONTROL.init([0; 64]),
    );

    let (ep_out, ep_in) = {
        let mut function = builder.function(0xff, 0, 0);
        let mut interface = function.interface();
        let mut alt = interface.alt_setting(0xff, 0, 0, None);
        let ep_out = alt.endpoint_bulk_out(None, PACKET as u16);
        let ep_in = alt.endpoint_bulk_in(None, PACKET as u16);
        (ep_out, ep_in)
    };
    builder.handler(HANDLER.init(GsUsbHandler));

    let mut device = builder.build();
    let (can_tx, can_rx, _props) = ports.can.split();

    join4(
        device.run(),
        usb_rx_task(ep_out),
        usb_tx_task(ep_in),
        join(can_rx_task(can_rx), can_tx_task(can_tx)),
    )
    .await;
}

//#[embassy_executor::task]
async fn usb_rx_task(mut ep_out: <Usb as Driver<'static>>::EndpointOut) {
    let mut buf = [0u8; PACKET];
    loop {
        ep_out.wait_enabled().await;
        if let Ok(n) = ep_out.read(&mut buf).await {
            if let Some(frame) = parse_host_frame(&buf[..n]) {
                // 基础任务只支持标准数据帧
                if frame.can_id & gs_usb::GS_EFF_FLAG != 0 {
                    continue;
                }
                let _ = CAN_TX_CHANNEL.try_send(frame);
            }
        }
    }
}

//#[embassy_executor::task]
async fn usb_tx_task(mut ep_in: <Usb as Driver<'static>>::EndpointIn) {
    loop {
        ep_in.wait_enabled().await;
        let frame = match select(CAN_RX_CHANNEL.receive(), ECHO_CHANNEL.receive()).await {
            Either::First(f) => f,
            Either::Second(f) => f,
        };
        let mut buf = [0u8; PACKET];
        let n = write_host_frame(&frame, &mut buf);
        if n > 0 {
            let _ = ep_in.write(&buf[..n]).await;
        }
    }
}

// async fn usb_tx_task(mut usb_tx: <Usb as ...>::EndpointIn) {
//     loop {
//         let frame = CAN_RX_CHANNEL.receive().await;
//         let text = protocol::format_rx(&frame); // "RX <ID> <DATA>\n"
//         let _ = usb_tx.write_packet(text.as_bytes()).await;
//     }
// }

/// FDCAN RX → CAN RX queue.
async fn can_rx_task(mut rx: CanRx<'static>) {
    loop {
        let envelope = match rx.read_fd().await {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !CAN_STARTED.load(core::sync::atomic::Ordering::Acquire) {
            continue;
        }
        let frame = envelope.frame;
        let mut gs = GsHostFrame::empty_rx();
        match frame.id() {
            Id::Standard(id) => {
                gs.can_id = id.as_raw() as u32 & GS_SFF_MASK;
            }
            Id::Extended(_) => continue,
        }
        if frame.is_remote_frame() {
            continue;
        }
        let data = frame.data();
        let len = data.len().min(8);
        gs.can_dlc = len as u8;
        gs.data[..len].copy_from_slice(&data[..len]);
        let _ = CAN_RX_CHANNEL.try_send(gs);
    }
}

/// CAN TX queue → FDCAN, then echo back on USB with original echo_id.
async fn can_tx_task(mut tx: CanTx<'static>) {
    loop {
        let gs = CAN_TX_CHANNEL.receive().await;
        if !CAN_STARTED.load(core::sync::atomic::Ordering::Acquire) {
            continue;
        }
        let len = (gs.can_dlc as usize).min(8);
        let id = (gs.can_id & GS_SFF_MASK) as u16;
        if let Ok(frame) = FdFrame::new_standard(id, &gs.data[..len]) {
            if tx.write_fd(&frame).await.is_some() {
                let _ = ECHO_CHANNEL.try_send(gs);
            }
        }
    }
}
