#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{gpio::{Level, Output, Speed, Input, Pull}, Config, peripherals::{PA8, PA6, PA7, PB13, PB11, FDCAN1, PA11, PA12 }, bind_interrupts, can};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};
mod can_exocet;
use can_exocet::{Priority, Subsystem};

static mut ERROR_FLAG: bool = false;

     bind_interrupts!(struct Irqs {
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

#[derive(Debug)]
enum State {
    Idle,
    Active,
    Init,
    Shutdown,
    Error
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hsi = true;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL85,
            divp: None,
            divq: None,
            // Main system clock at 170 MHz
            divr: Some(PllRDiv::DIV2),
        });
        config.rcc.sys = Sysclk::PLL1_R;
    }
    let p = embassy_stm32::init(config);

    spawner.spawn(state_machine(spawner, p.PA8, p.PA6, p.PA7, p.PB13, p.PB11, p.FDCAN1, p.PA11, p.PA12)).unwrap(); // Tâche: Machine à états
}

#[embassy_executor::task]
async fn state_machine(spawner: Spawner, pin_a8: PA8, pin_a6: PA6, pin_a7: PA7, pin_b13: PB13, pin_b11: PB11, pin_fdcan1: FDCAN1, pin_a11: PA11, pin_a12: PA12) {

    let mut led_g = Output::new(pin_a8, Level::High, Speed::Low); //Green LED
    let mut led_y = Output::new(pin_a6, Level::High, Speed::Low); //Yellow LED
    let mut led_r = Output::new(pin_a7, Level::High, Speed::Low); //Red LED

    let button_g = Input::new(pin_b13, Pull::Down); //Green Button
    let button_r = Input::new(pin_b11, Pull::Down); //Red Button

    let can = can::CanConfigurator::new(pin_fdcan1, pin_a11, pin_a12, Irqs);
    let mut can = can.start(can::OperatingMode::NormalOperationMode);

    let mut last_read_ts = embassy_time::Instant::now();

    let mut state = State::Idle;
    loop { 

        if unsafe { ERROR_FLAG } {
            state = State::Error;
        }

        match state{
            State::Idle=>{


                if button_g.is_high(){
                    state = State::Init;
                }

                led_r.set_high();
                Timer::after_millis(100).await;
            }

            State::Init=>{

                info!("Initialisation des composants");
                led_r.set_low();
                led_g.set_low();
                led_y.set_high();

                Timer::after_millis(3000).await;

                let frame = can::frame::Frame::new_extended(Subsystem::Broadcast.to_u32(), &[0; 8]).unwrap();
                info!("Writing frame");

                _ = can.write(&frame).await;

                match can.read().await {
                    Ok(envelope) => {
                        let (ts, rx_frame) = (envelope.ts, envelope.frame);
                        let delta = (ts - last_read_ts).as_millis();
                        last_read_ts = ts;

                        state = State::Active;

                        info!(
                            "Rx: {} {:02x} --- {}ms",
                            rx_frame.header().len(),
                            rx_frame.data()[0..rx_frame.header().len() as usize],
                            delta,
                        )
                    }
                    Err(_err) => error!("Error in frame"),
                }

            }

            State::Active=>{

                if button_r.is_high(){
                    state = State::Shutdown;
                }

                info!("Système actif");
                led_g.set_high();
                led_y.set_low();
                led_r.set_low(); 

            }

            State::Shutdown=>{
                info!("Arrêt du système");
                led_g.set_low();
                led_y.set_high();
                led_r.set_low();

                Timer::after_millis(3000).await;

                state = State::Idle;
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