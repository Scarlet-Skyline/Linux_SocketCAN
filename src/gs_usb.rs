//! src/gs_usb.rs  —  任务3新建文件
//! Minimal gs_usb protocol definitions, aligned with Linux gs_usb.c.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU32};

// ---- Control request codes ----
pub const GS_USB_BREQ_HOST_FORMAT: u8 = 0;
pub const GS_USB_BREQ_BITTIMING: u8 = 1;
pub const GS_USB_BREQ_MODE: u8 = 2;
pub const GS_USB_BREQ_BERR: u8 = 3;
pub const GS_USB_BREQ_BT_CONST: u8 = 4;
pub const GS_USB_BREQ_DEVICE_CONFIG: u8 = 5;

// ---- Mode ----
pub const GS_CAN_MODE_RESET: u32 = 0;
pub const GS_CAN_MODE_START: u32 = 1;

// ---- Feature bits ----
pub const GS_CAN_FEATURE_LISTEN_ONLY: u32 = 1 << 0;
pub const GS_CAN_FEATURE_LOOP_BACK: u32 = 1 << 1;

// ---- CAN ID flags (Linux) ----
pub const GS_EFF_FLAG: u32 = 0x8000_0000;
pub const GS_RTR_FLAG: u32 = 0x4000_0000;
pub const GS_ERR_FLAG: u32 = 0x2000_0000;
pub const GS_SFF_MASK: u32 = 0x0000_07FF;

// ---- Frame flags ----
pub const GS_CAN_FLAG_FD: u8 = 1 << 0;
pub const GS_CAN_FLAG_BRS: u8 = 1 << 1;

// RX frames use this echo_id.
pub const GS_HOST_FRAME_ECHO_ID_RX: u32 = 0xFFFF_FFFF;

/// Compact gs_host_frame for Classical CAN (8 data bytes max).
/// Size = 4 + 4 + 1 + 1 + 1 + 1 + 8 = 20 bytes.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct GsHostFrame {
    pub echo_id: u32,
    pub can_id: u32,
    pub can_dlc: u8,
    pub channel: u8,
    pub flags: u8,
    pub reserved: u8,
    pub data: [u8; 8],
}

impl GsHostFrame {
    pub const fn empty_rx() -> Self {
        Self {
            echo_id: GS_HOST_FRAME_ECHO_ID_RX,
            can_id: 0,
            can_dlc: 0,
            channel: 0,
            flags: 0,
            reserved: 0,
            data: [0; 8],
        }
    }
}

/// gs_device_bt_const — returned for GS_USB_BREQ_BT_CONST.
#[repr(C, packed)]
pub struct GsDeviceBtConst {
    pub feature: u32,
    pub fclk_can: u32,
    pub tseg1_min: u32,
    pub tseg1_max: u32,
    pub tseg2_min: u32,
    pub tseg2_max: u32,
    pub sjw_max: u32,
    pub brp_min: u32,
    pub brp_max: u32,
    pub brp_inc: u32,
}

pub const fn bt_const() -> GsDeviceBtConst {
    GsDeviceBtConst {
        feature: GS_CAN_FEATURE_LISTEN_ONLY | GS_CAN_FEATURE_LOOP_BACK,
        fclk_can: 80_000_000,
        tseg1_min: 1,
        tseg1_max: 256,
        tseg2_min: 1,
        tseg2_max: 128,
        sjw_max: 128,
        brp_min: 1,
        brp_max: 512,
        brp_inc: 1,
    }
}

/// gs_device_bittiming — received for GS_USB_BREQ_BITTIMING.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct GsDeviceBittiming {
    pub prop_seg: u32,
    pub phase_seg1: u32,
    pub phase_seg2: u32,
    pub sjw: u32,
    pub brp: u32,
}

/// gs_device_mode — received for GS_USB_BREQ_MODE.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct GsDeviceMode {
    pub mode: u32,
    pub flags: u32,
}

/// gs_device_config — returned for GS_USB_BREQ_DEVICE_CONFIG.
#[repr(C, packed)]
pub struct GsDeviceConfig {
    pub reserved: [u8; 3],
    pub icount: u8,
    pub sw_version: u32,
    pub hw_version: u32,
}

pub const DEVICE_CONFIG: GsDeviceConfig = GsDeviceConfig {
    reserved: [0; 3],
    icount: 1,
    sw_version: 1,
    hw_version: 1,
};

// Shared state written by control requests, read by CAN tasks.
pub static CAN_STARTED: AtomicBool = AtomicBool::new(false);
pub static NOMINAL_BRP: AtomicU32 = AtomicU32::new(1);
pub static NOMINAL_TSEG1: AtomicU32 = AtomicU32::new(59);
pub static NOMINAL_TSEG2: AtomicU32 = AtomicU32::new(20);
pub static NOMINAL_SJW: AtomicU32 = AtomicU32::new(20);

/// Parse one gs_host_frame from a USB OUT packet.
pub fn parse_host_frame(buf: &[u8]) -> Option<GsHostFrame> {
    let n = core::mem::size_of::<GsHostFrame>();
    if buf.len() < n {
        return None;
    }
    let mut frame = GsHostFrame::empty_rx();
    unsafe {
        core::ptr::copy_nonoverlapping(
            buf.as_ptr(),
            &mut frame as *mut GsHostFrame as *mut u8,
            n,
        );
    }
    Some(frame)
}

/// Serialize one gs_host_frame into `buf`. Returns bytes written.
pub fn write_host_frame(frame: &GsHostFrame, buf: &mut [u8]) -> usize {
    let n = core::mem::size_of::<GsHostFrame>();
    if buf.len() < n {
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            frame as *const GsHostFrame as *const u8,
            buf.as_mut_ptr(),
            n,
        );
    }
    n
}