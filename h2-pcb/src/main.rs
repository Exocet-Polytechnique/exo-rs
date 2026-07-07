#![no_std]
#![no_main]

// include pub mod libs
pub mod tasks;

use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_stm32::{Config, Peri, adc::{Adc, AdcChannel, AnyAdcChannel, SampleTime}, can, gpio::{Input, Level, Output, Speed}, i2c::{self, I2c, Config as I2cConfig}, peripherals::{ADC1, ADC2, FDCAN1, PB4, PB5, PB6, PB9, PC1, PC2, PC3, PC6, PC7}};
use embassy_sync::{blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex}, channel::Channel, priority_channel::PriorityChannel, watch::Watch};
use embassy_time::Timer;
use embedded_hal::can::{Id, StandardId};
use heapless::binary_heap::Min;
use {defmt_rtt as _, panic_probe as _};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, peripherals};
use exo_drivers::{
    ds2484::{DS2484},
    ds18b20::{self, DS18B20}
};
use crate::tasks::can::can_task;

pub mod dbc_gen {
    include!(concat!(env!("OUT_DIR"), "/dbc_gen.rs"));
}

bind_interrupts!(struct Irqs {
    DMA1_CHANNEL6 => dma::InterruptHandler<peripherals::DMA1_CH6>;
    DMA1_CHANNEL7 => dma::InterruptHandler<peripherals::DMA1_CH7>;
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
    I2C2_EV => i2c::EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => i2c::ErrorInterruptHandler<peripherals::I2C2>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {

    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000), // 8-MHz oscillator
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV2,
            mul: PllMul::MUL85,
            divp: None,
            divq: Some(PllQDiv::DIV8), // 42.5 MHz for CAN
            divr: Some(PllRDiv::DIV2), // Main system clock at 170 MHz
        });
        config.rcc.mux.adc12sel = mux::Adcsel::SYS;
        config.rcc.mux.fdcansel = mux::Fdcansel::PLL1_Q;
        config.rcc.sys = Sysclk::PLL1_R;
    }
    let p = embassy_stm32::init(config);

    let mut can_config = can::CanConfigurator::new(p.FDCAN1, p.PA11, p.PA12, Irqs);
    can_config.properties().set_extended_filter(can::filter::ExtendedFilterSlot::_0, can::filter::ExtendedFilter::accept_all_into_fifo1());
    can_config.set_bitrate(250_000);

    let can_bus: can::Can<'static> = can_config.start(can::OperatingMode::NormalOperationMode);

    spawner.spawn(unwrap!(can_task(can_bus)));
}