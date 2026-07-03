#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    can::{self, Frame, filter::{StandardFilter, StandardFilterSlot, FilterType, Action}},
    gpio::{Input, Level, Output, Pull, Speed},
    peripherals::{FDCAN1, PA6, PA7, PA8, PA11, PA12, PB11, PB13},
};
use embassy_futures::select::{select, Either};
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
    IDLE,
    STARTING,
    RUNNING,
    SHUTDOWN,
    FAULT,
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.mux.fdcansel = mux::Fdcansel::PCLK1;
    }
    let p = embassy_stm32::init(config);

    spawner
        .spawn(cockpit(
            spawner, p.PA8, p.PA6, p.PA7, p.PB13, p.PB11, p.FDCAN1, p.PA11, p.PA12,
        ))
        .unwrap();

    loop {
        Timer::after_millis(1000).await;
    }
}

#[embassy_executor::task]
async fn cockpit(
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

    let button_g = Input::new(pin_b13, Pull::Down); //Green Button
    let button_r = Input::new(pin_b11, Pull::Down); //Red Button

    let mut can = can::CanConfigurator::new(pin_fdcan1, pin_a11, pin_a12, Irqs);
    can.set_bitrate(250_000);
    // Slot 0: accept only frames from DriverInterface (module 4)
    // Mask 0x00E7 checks bits 7-5 (module index) and bits 2-0 (constant)
    can.properties().set_standard_filter(
        StandardFilterSlot::_0,
        StandardFilter {
            filter: FilterType::BitMask {
                filter: 103_u16,
                mask: 0x00E7,
            },
            action: Action::StoreInFifo0,
        },
    );
    // Slot 1: catch-all rejected frames not matched by slot 0 are discarded
    can.properties().set_standard_filter(
        StandardFilterSlot::_1,
        StandardFilter {
            filter: FilterType::BitMask {
                filter: 0_u16,
                mask: 0_u16,
            },
            action: Action::Reject,
        },
    );
    let mut can: can::Can<'_> = can.start(can::OperatingMode::NormalOperationMode);

    let mut state = State::IDLE;
    loop {
        if unsafe { ERROR_FLAG } {
            state = State::FAULT;
        }

        match state {
            State::IDLE => {
                led_r.set_high();
                led_g.set_low();
                led_y.set_low();

                if button_g.is_high() {
                    if let Ok(frame) = dbc_gen::FrameP1i::new(
                        Module::DriverInterface as u8,
                        0,
                        Instructions::Start as u64,
                    ) {
                        if let Ok(f) = Frame::new_standard(dbc_gen::FrameP1i::MESSAGE_ID as u16, frame.raw()) {
                            can.write(&f).await;
                            info!("Start instruction sent to Dashboard");
                            state = State::STARTING;
                        }
                    }
                }

                Timer::after_millis(50).await;
            }

            State::STARTING => {
                match select(can.read(), Timer::after_millis(300)).await { // No explicit buffer needed, added filter to accept only frames from DriverInterface (module 4) so it doesnt get flooded with other frames
                    Either::First(Ok(envelope)) => {
                        let id: u32 = match envelope.frame.id() {
                            embedded_can::Id::Standard(id) => id.as_raw() as u32,
                            _ => 0,
                        };
                        if id == dbc_gen::FrameP4i::MESSAGE_ID {
                            if let Ok(p4i) = dbc_gen::FrameP4i::try_from(envelope.frame.data()) {
                                if p4i.instruction() == 0x41 {
                                    info!("Start ACK received");
                                    led_g.set_low();
                                    state = State::RUNNING;
                                }
                            }
                        }
                    }
                    Either::First(Err(_)) => error!("CAN error in STARTING"),
                    Either::Second(_) => {
                        led_g.toggle();
                        led_r.set_low();
                        led_y.set_low();
                    }
                }
            }

            State::RUNNING => {
                led_g.set_high();
                led_r.set_low();
                led_y.set_low();

                if button_r.is_high() {
                    if let Ok(frame) = dbc_gen::FrameP1i::new(
                        Module::DriverInterface as u8,
                        0,
                        Instructions::Shutdown as u64,
                    ) {
                        if let Ok(f) = Frame::new_standard(dbc_gen::FrameP1i::MESSAGE_ID as u16, frame.raw()) {
                            can.write(&f).await;
                            info!("Shutdown instruction sent to Dashboard");
                            state = State::SHUTDOWN;
                        }
                    }
                }

                Timer::after_millis(50).await;
            }

            State::SHUTDOWN => {
                match select(can.read(), Timer::after_millis(300)).await {
                    Either::First(Ok(envelope)) => {
                        let id: u32 = match envelope.frame.id() {
                            embedded_can::Id::Standard(id) => id.as_raw() as u32,
                            _ => 0,
                        };
                        if id == dbc_gen::FrameP4i::MESSAGE_ID {
                            if let Ok(p4i) = dbc_gen::FrameP4i::try_from(envelope.frame.data()) {
                                if p4i.instruction() == 0x42 {
                                    info!("Shutdown ACK received");
                                    led_r.set_low();
                                    state = State::IDLE;
                                }
                            }
                        }
                    }
                    Either::First(Err(_)) => error!("CAN error in SHUTDOWN"),
                    Either::Second(_) => {
                        led_r.toggle();
                        led_g.set_low();
                        led_y.set_low();
                    }
                }
            }

            State::FAULT => {
                led_g.set_low();
                led_y.set_low();
                led_r.toggle();
                info!("Erreur détectée, clignotement de la LED rouge");
                Timer::after_millis(10).await;
            }
        }
    }
}

// To test out the sending of CAN frames

// #[embassy_executor::task]
// async fn sender(pin_fdcan1: FDCAN1, pin_a11: PA11, pin_a12: PA12) {
//     info!("Démarrage du système...");
//
//     let mut can = can::CanConfigurator::new(pin_fdcan1, pin_a11, pin_a12, Irqs);
//     can.set_bitrate(250_000);
//     let mut can: can::Can<'_> = can.start(can::OperatingMode::NormalOperationMode);
//
//     loop {
//         let frame = match dbc_gen::FrameP1d::new(
//             0 as u8,
//             0 as u8,
//             0x01_u8, // DataType::Speed
//             (18 as f32).to_bits(),
//         ) {
//             Ok(f) => f,
//             Err(_e) => {
//                 info!("Failed to create CAN frame");
//                 Timer::after_millis(1000).await;
//                 continue;
//             }
//         };
//
//         match Frame::new_standard(dbc_gen::FrameP1d::MESSAGE_ID as u16, frame.raw()) {
//             Ok(f) => {
//                 can.write(&f).await;
//                 info!("Can frame sent with ID: {}", dbc_gen::FrameP1d::MESSAGE_ID);
//             }
//             Err(e) => {
//                 info!("Invalid frame: {:?}", e);
//             }
//         }
//
//         Timer::after_millis(3000).await;
//     }
// }

// To test out the filter configuration

// #[embassy_executor::task]
// async fn test_filter(pin_fdcan1: FDCAN1, pin_a11: PA11, pin_a12: PA12) {
//     info!("=== Filter Test ===");

//     let mut can = can::CanConfigurator::new(pin_fdcan1, pin_a11, pin_a12, Irqs);
//     can.set_bitrate(250_000);
//     can.properties().set_standard_filter(
//         StandardFilterSlot::_0,
//         StandardFilter {
//             filter: FilterType::BitMask {
//                 filter: 103_u16,
//                 mask: 0x00E7,
//             },
//             action: Action::StoreInFifo0,
//         },
//     );
//     // Catch-all reject: frames not matched by slot 0 are rejected (mask=0 matches everything)
//     can.properties().set_standard_filter(
//         StandardFilterSlot::_1,
//         StandardFilter {
//             filter: FilterType::BitMask {
//                 filter: 0_u16,
//                 mask: 0_u16,
//             },
//             action: Action::Reject,
//         },
//     );
//     let mut can = can.start(can::OperatingMode::InternalLoopbackMode);

//     let data = [0u8; 8];

//     // Test 1: P4I (DriverInterface, ID 111) — should pass the filter
//     if let Ok(f) = Frame::new_standard(dbc_gen::FrameP4i::MESSAGE_ID as u16, &data) {
//         can.write(&f).await;
//         info!("Sent P4I (ID {}), expecting RECEIVED", dbc_gen::FrameP4i::MESSAGE_ID);
//     }
//     match select(can.read(), Timer::after_millis(100)).await {
//         Either::First(Ok(env)) => {
//             let id = match env.frame.id() {
//                 embedded_can::Id::Standard(id) => id.as_raw(),
//                 _ => 0,
//             };
//             info!("[PASS] Test 1: received frame ID {}", id);
//         }
//         Either::First(Err(_)) => error!("[FAIL] Test 1: CAN error"),
//         Either::Second(_)     => error!("[FAIL] Test 1: timeout — P4I was incorrectly filtered"),
//     }

//     Timer::after_millis(20).await;

//     // Test 2: P1I (Cockpit, not module 4) — should be blocked by the filter
//     if let Ok(f) = Frame::new_standard(dbc_gen::FrameP1i::MESSAGE_ID as u16, &data) {
//         can.write(&f).await;
//         info!("Sent P1I (ID {}), expecting FILTERED", dbc_gen::FrameP1i::MESSAGE_ID);
//     }
//     match select(can.read(), Timer::after_millis(100)).await {
//         Either::First(Ok(env)) => {
//             let id = match env.frame.id() {
//                 embedded_can::Id::Standard(id) => id.as_raw(),
//                 _ => 0,
//             };
//             error!("[FAIL] Test 2: received frame ID {} — filter did not block it", id);
//         }
//         Either::First(Err(_)) => error!("[FAIL] Test 2: CAN error"),
//         Either::Second(_)     => info!("[PASS] Test 2: timeout — P1I correctly filtered"),
//     }

//     info!("=== Filter Test Complete ===");
//     loop { Timer::after_millis(1000).await; }
// }

#[repr(u8)]
enum Module {
    // Broadcast        = 0b1111,
    // Cockpit          = 0b0001,
    // Hydrogen         = 0b0010,
    // HighPower        = 0b0011,
    DriverInterface  = 0b0100,
    // TelemetryBattery = 0b0101,
}

#[repr(u64)]
enum Instructions {
    Start    = 0x01, // From Cockpit to DriverInterface: Instruction to indicate that the system started
    Shutdown = 0x02, // From Cockpit to DriverInterface: Instruction to indicate that the system is shutting down
    ValidationForStartingState = 0x41, // From DriverInterface to Cockpit: Validation to indicate that the system is ready to start and pass into the RUNNING state
    ValidationForShutdownState = 0x42, // From DriverInterface to Cockpit: Validation to indicate that the system is ready to shutdown and pass into the IDLE state
}