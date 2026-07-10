#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts, can::{self, Frame}, gpio::{Input, Level, Output, Pull, Speed}, peripherals::FDCAN1
};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

enum State {
    Stopped,
    Running,
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.mux.fdcansel = mux::Fdcansel::PCLK1;
    }
    let p = embassy_stm32::init(config);

    let mut configurator = can::CanConfigurator::new(p.FDCAN1, p.PA11, p.PA12, Irqs);
    configurator.set_bitrate(250_000);

    let mut can: can::Can<'static> = configurator.start(can::OperatingMode::NormalOperationMode);

    let start_button = Input::new(p.PB13, Pull::Down);
    let stop_button = Input::new(p.PB11, Pull::Down);

    let mut led_g = Output::new(p.PA8, Level::Low, Speed::Low);
    let mut led_r = Output::new(p.PA7, Level::High, Speed::Low);

    let mut current_state = State::Stopped;

    let start_frame = Frame::new_standard(0x00, &[0xFA, 0xCE, 0xFA, 0xCE]).unwrap();
    let stop_frame = Frame::new_standard(0x00, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

    loop {
        match current_state {
            State::Stopped => {
                if start_button.is_high() {
                    led_r.set_low();
                    for _ in 0..3 {
                        can.write(&start_frame).await;
                        Timer::after_millis(100).await;
                    }
                    current_state = State::Running;
                    led_g.set_high();
                }
            }
            State::Running => {
                if stop_button.is_high() {
                    led_g.set_low();
                    for _ in 0..3 {
                        can.write(&stop_frame).await;
                        Timer::after_millis(100).await;
                    }
                    current_state = State::Stopped;
                    led_r.set_high();
                }
            }
        }
        Timer::after_millis(10).await;
    }
}
