#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{bind_interrupts, can, gpio::{Input, Level, Output, Pull, Speed}, peripherals::{FDCAN1, PA11, PA12, PA6, PA7, PA8, PB11, PB13 }, time::Hertz, Config};
use embassy_time::Timer;
use crate::can_exocet::{enums::{DataSubtype, FrameType}, interface::CanFrameExocet};

use {defmt_rtt as _, panic_probe as _};

mod can_exocet;
use can_exocet::interface::CanDriver;
use can_exocet::enums::{Priority, Subsystem};

static mut ERROR_FLAG: bool = false;

     bind_interrupts!(struct Irqs {
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

#[derive(Debug)]
enum State {
    Idle,
    Send,
    Receive,
    Error
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(24_000_000),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV6,
            mul: PllMul::MUL85,
            divp: None,
            divq: Some(PllQDiv::DIV8), // 42.5 Mhz for fdcan.
            divr: Some(PllRDiv::DIV2), // Main system clock at 170 MHz
        });
        config.rcc.mux.fdcansel = mux::Fdcansel::PLL1_Q;
        config.rcc.sys = Sysclk::PLL1_R;
    }
    let p = embassy_stm32::init(config);

    spawner.spawn(state_machine(spawner, p.PA8, p.PA6, p.PA7, p.PB13, p.PB11, p.FDCAN1, p.PA11, p.PA12)).unwrap(); // Tâche: Machine à états
}

#[embassy_executor::task]
async fn state_machine(_spawner: Spawner, pin_a8: PA8, pin_a6: PA6, pin_a7: PA7, pin_b13: PB13, pin_b11: PB11, pin_fdcan1: FDCAN1, pin_a11: PA11, pin_a12: PA12) {

    let mut led_g = Output::new(pin_a8, Level::High, Speed::Low); //Green LED
    let mut led_y = Output::new(pin_a6, Level::High, Speed::Low); //Yellow LED
    let mut led_r = Output::new(pin_a7, Level::High, Speed::Low); //Red LED

    let button_g = Input::new(pin_b13, Pull::Down); //Green Button
    let button_r = Input::new(pin_b11, Pull::Down); //Red Button

    let mut can = can::CanConfigurator::new(pin_fdcan1, pin_a11, pin_a12, Irqs);
    can.set_bitrate(250_000);
    let can = can.start(can::OperatingMode::NormalOperationMode);

    let mut state = State::Idle;

    let mut can_driver = CanDriver::new(can);
    loop { 

        if unsafe { ERROR_FLAG } {
            state = State::Error;
        }

        match state{
            State::Idle=>{


                if button_g.is_high(){
                    state = State::Send;
                }

                if button_r.is_high(){
                    state = State::Receive;
                }

                led_r.set_high();
                Timer::after_millis(100).await;
            }

            State::Send=>{

                info!("Sending Mode");
                led_r.set_low();
                led_g.set_high();
                led_y.set_low();

                Timer::after_millis(3000).await;
                info!("Writing frame");

                loop{
                    let can_frame = CanFrameExocet::new(FrameType::Data(DataSubtype::DataRequest), 3).unwrap();
                    can_driver.send_message(Priority::CriticalErrorMessage, Subsystem::Broadcast,  3, true, can_frame).await.unwrap();
                }
            }

            State::Receive=>{

                info!("Receiving Mode");

                info!("Système actif");
                led_g.set_low();
                led_y.set_high();
                led_r.set_low(); 

                loop{
                    let (frame, ts) = can_driver.read_message().await.unwrap();
                    info!("Received Header: {:?}", frame.header());
                    info!("Received Data: {:?}", frame.data());
                    info!("Received Timestamp: {:?}", ts);
                }

            }

            State::Error=>{

                led_g.set_low();
                led_y.set_low();
                if led_r.is_set_high() {
                    led_r.set_low();
                } else {
                    led_r.set_high();
                }
                info!("Erreur détectée, clignotement de la LED rouge");
                Timer::after_millis(10).await;
            }
        }
        Timer::after_millis(10).await;
     }
}