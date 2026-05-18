#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    can::{self, Frame},
    gpio::{Input, Level, Output, Pull, Speed},
    peripherals::{FDCAN1, PA6, PA7, PA8, PA11, PA12, PB11, PB13},
    time::Hertz,
};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

pub mod dbc_gen {
    include!(concat!(env!("OUT_DIR"), "/dbc_gen.rs"));
}

static mut ERROR_FLAG: bool = false;

bind_interrupts!(struct Irqs {
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

#[derive(Debug)]
enum State {
    Idle,
    Starting,
    Active,
    Shutdown,
    Fault,
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

    spawner
        .spawn(state_machine(
            spawner, p.PA8, p.PA6, p.PA7, p.PB13, p.PB11, p.FDCAN1, p.PA11, p.PA12,
        ))
        .unwrap(); // Tâche: Machine à états
}

#[embassy_executor::task]
async fn state_machine(
    _spawner: Spawner,
    pin_a8: PA8,
    pin_a6: PA6,
    pin_a7: PA7,
    pin_b13: PB13,
    pin_b11: PB11,
    pin_fdcan1: FDCAN1,
    pin_a11: PA11,
    pin_a12: PA12,
) {
    info!("Démarrage du système...");

    let mut led_g = Output::new(pin_a8, Level::Low, Speed::Low); //Green LED
    let mut led_y = Output::new(pin_a6, Level::Low, Speed::Low); //Yellow LED
    let mut led_r = Output::new(pin_a7, Level::Low, Speed::Low); //Red LED

    let button_g = Input::new(pin_b13, Pull::Up); //Green Button
    let button_r = Input::new(pin_b11, Pull::Up); //Red Button

    let mut can = can::CanConfigurator::new(pin_fdcan1, pin_a11, pin_a12, Irqs);
    can.set_bitrate(250_000);
    let mut can: can::Can<'_> = can.start(can::OperatingMode::NormalOperationMode);

    let mut state = State::Idle;
    loop {
        if unsafe { ERROR_FLAG } {
            state = State::Fault;
        }

        match state {
            State::Idle => {
                info!("Idle Mode");
                
                led_g.set_low();
                led_y.set_low();

                if button_g.is_low() {
                    state = State::Starting;
                }

                if led_r.is_set_high() {
                    led_r.set_low();
                } else {
                    led_r.set_high();
                }
                Timer::after_millis(500).await;
            }

            State::Starting => {
                info!("Starting Mode");
                led_y.set_high();
                led_g.set_low();
                led_r.set_low();

                let frame = match dbc_gen::Nci::new(
                    Module::Dashboard as u8,
                    Module::Cockpit as u8,
                    Instructions::ValveVerification as u32,
                ) {
                    Ok(f) => f,
                    Err(_) => {
                        error!("Failed to create CAN frame");
                        return;
                    }
                };

                can.write(&Frame::new_standard(dbc_gen::Nci::MESSAGE_ID as u16, frame.raw()).unwrap()).await;

                loop {
                    match can.read().await {
                        Ok(envelope) => {
                            let id: u32 = match envelope.frame.id() {
                                embedded_can::Id::Standard(id) => id.as_raw() as u32,
                                _ => continue,
                            };
                            if id == dbc_gen::Ntd::MESSAGE_ID { // Assuming the response comes from the Telemetry module in this example
                                error!("Received valve verification response");
                                state = State::Active;
                                // Other verifications to come
                                break;
                            }   else {
                                error!("Received unrelated CAN frame with ID: {}", id);
                                state = State::Fault; 
                                break;
                            }
                        }
                        Err(_err) => error!("Error in frame"),
                    }
                }
                
            }
            State::Active => {
                info!("Active Mode");
                led_g.set_high();
                led_y.set_low();
                led_r.set_low();

                if button_r.is_low() {
                    state = State::Shutdown;
                }

                Timer::after_millis(1000).await;
            }
            State::Shutdown => {
                info!("Shutdown Mode");
                // Send shutdown command to dashboard
            }
            State::Fault => {
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
    }
}

#[repr(u8)]
enum Module {
    Broadcast = 0b1111,
    Cockpit = 0b0000,
    Hydrogen = 0b0001,
    LowPower = 0b0010,
    HighPower = 0b0011,
    Dashboard = 0b0100,
    Telemetry = 0b0101,
}// Will need to update the DBC to add the missing modules

#[repr(u32)]
enum Instructions {
    NoInstruction = 0,
    ValveVerification = 1,
    ValveCalibration = 2,
}
