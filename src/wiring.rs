//! Complete the two TODOs in this file.

use embassy_stm32::{
    Peripherals,
    can::Can,
    //gpio::{Flex, Level, Output, Speed}, //去掉了有关swd调试的部分
    //usart::{BufferedUart, Config},
};
//use static_cell::StaticCell;

//use crate::swd::Swd;

pub struct Ports {
    pub can: Can<'static>,
    //pub uart: Option<BufferedUart<'static>>,
}

pub fn configure(p: Peripherals) -> (crate::board::Usb, Ports) {
    let usb = crate::board::usb(p.USB_OTG_HS, p.PA12, p.PA11);
    let can = crate::board::can1(p.FDCAN1, p.PD0, p.PD1);
    
    (usb, Ports { can })
}