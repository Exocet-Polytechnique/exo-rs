#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    can::{self, Frame, filter::{StandardFilter, StandardFilterSlot, FilterType, Action}},
    gpio::{Input, Level, Output, Pull, Speed},
    peripherals::{FDCAN1, PA6, PA7, PA8, PB11, PB13},
};
use embassy_futures::select::{select, Either};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

pub mod dbc_gen {
    include!(concat!(env!("OUT_DIR"), "/dbc_gen.rs"));
}

static mut ERROR_FLAG: bool = false;

static CAN_CHANNEL: Channel<CriticalSectionRawMutex, CanEvent, 4> = Channel::new();

bind_interrupts!(struct Irqs {
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

#[derive(Debug)]
enum State {
    IDLE,
    STARTING,
    RUNNING,
    FAULT,
}

// External events received from the DriverInterface over CAN. These are sent to the cockpit task via CAN_CHANNEL.
#[derive(Clone, Copy)]
enum CanEvent {
    ValidationForStartingState,
    ValidationForShutdownState,
    RequestShutdown,
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.mux.fdcansel = mux::Fdcansel::PCLK1;
    }
    let p = embassy_stm32::init(config);

    let mut configurator = can::CanConfigurator::new(p.FDCAN1, p.PA11, p.PA12, Irqs);
    configurator.set_bitrate(250_000);

    // Configure CAN filter to accept only frames from DriverInterface (module 4) and reject all others.
    // Slot 0: accept only frames from DriverInterface (module 4)
    // Mask 0x00E7 checks bits 7-5 (module index) and bits 2-0 (constant)
    configurator.properties().set_standard_filter(
        StandardFilterSlot::_0,
        StandardFilter {
            filter: FilterType::BitMask { filter: 103_u16, mask: 0x00E7 },
            action: Action::StoreInFifo0,
        },
    );
    // Slot 1: catch-all reject — frames not matched by slot 0 are discarded
    configurator.properties().set_standard_filter(
        StandardFilterSlot::_1,
        StandardFilter {
            filter: FilterType::BitMask { filter: 0_u16, mask: 0_u16 },
            action: Action::Reject,
        },
    );
    let can: can::Can<'static> = configurator.start(can::OperatingMode::NormalOperationMode);
    let (tx, rx, _props) = can.split();

    spawner.spawn(can_reader(rx)).unwrap();
    spawner
        .spawn(cockpit(spawner, p.PA8, p.PA6, p.PA7, p.PB13, p.PB11, tx))
        .unwrap();

    loop {
        Timer::after_millis(1000).await;
    }
}

/// Reads all incoming CAN frames and forwards parsed events to CAN_CHANNEL.
#[embassy_executor::task]
async fn can_reader(mut rx: can::CanRx<'static>) {
    loop {
        match rx.read().await {
            Ok(envelope) => {
                let id = match envelope.frame.id() {
                    embedded_can::Id::Standard(id) => id.as_raw() as u32,
                    _ => continue,
                };
                if id == dbc_gen::FrameP4i::MESSAGE_ID {
                    if let Ok(p4i) = dbc_gen::FrameP4i::try_from(envelope.frame.data()) {
                        let instr = p4i.instruction();
                        if instr == Instructions::ValidationForStartingState as u64 {
                            CAN_CHANNEL.send(CanEvent::ValidationForStartingState).await;
                        } else if instr == Instructions::ValidationForShutdownState as u64 {
                            CAN_CHANNEL.send(CanEvent::ValidationForShutdownState).await;
                        } else if instr == Instructions::RequestShutdown as u64 {
                            CAN_CHANNEL.send(CanEvent::RequestShutdown).await;
                        }
                    }
                }
            }
            Err(_) => error!("CAN read error"),
        }
    }
}

/// Normal shutdown initiated by the cockpit button.
/// Sends P1I Shutdown to DriverInterface and waits for ValidationForShutdownState before returning.
async fn normal_shutdown(
    tx: &mut can::CanTx<'static>,
    led_g: &mut Output<'_>,
    led_r: &mut Output<'_>,
    led_y: &mut Output<'_>,
) {
    if let Ok(frame) = dbc_gen::FrameP1i::new(
        Module::DriverInterface as u8,
        0,
        Instructions::Shutdown as u64,
    ) {
        if let Ok(f) = Frame::new_standard(dbc_gen::FrameP1i::MESSAGE_ID as u16, frame.raw()) {
            tx.write(&f).await;
            info!("Shutdown instruction sent to DriverInterface");
        }
    }
    loop {
        match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
            Either::First(CanEvent::ValidationForShutdownState) => {
                info!("Shutdown ACK received");
                break;
            }
            Either::First(_) => {}
            Either::Second(_) => {
                led_r.toggle();
                led_g.set_low();
                led_y.set_low();
            }
        }
    }
    led_r.set_low();
}

/// Force shutdown initiated by DriverInterface.
/// Immediately stops the current operation and resets LEDs for IDLE.
fn force_shutdown(led_g: &mut Output<'_>, led_r: &mut Output<'_>, led_y: &mut Output<'_>) {
    led_g.set_low();
    led_r.set_low();
    led_y.set_low();
    info!("Force shutdown by DriverInterface, returning to IDLE");
}

#[embassy_executor::task]
async fn cockpit(
    _spawner: Spawner,
    pin_a8: PA8,
    pin_a6: PA6,
    pin_a7: PA7,
    pin_b13: PB13,
    pin_b11: PB11,
    mut tx: can::CanTx<'static>,
) {
    info!("Démarrage du système...");

    let mut led_g = Output::new(pin_a8, Level::Low, Speed::Low);
    let mut led_y = Output::new(pin_a6, Level::Low, Speed::Low);
    let mut led_r = Output::new(pin_a7, Level::Low, Speed::Low);

    let button_g = Input::new(pin_b13, Pull::Down);
    let button_r = Input::new(pin_b11, Pull::Down);

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
                            tx.write(&f).await;
                            info!("Start instruction sent to DriverInterface");
                            state = State::STARTING;
                        }
                    }
                }

                Timer::after_millis(50).await;
            }

            State::STARTING => {
                match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
                    Either::First(CanEvent::ValidationForStartingState) => {
                        info!("Start ACK received, entering RUNNING");
                        led_g.set_low();
                        state = State::RUNNING;
                    }
                    Either::First(CanEvent::RequestShutdown) => {
                        force_shutdown(&mut led_g, &mut led_r, &mut led_y);
                        state = State::IDLE;
                    }
                    Either::First(_) => {} // other events in STARTING, ignore
                    Either::Second(_) => {
                        // No frame received: still waiting. blink green
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

                match select(CAN_CHANNEL.receive(), Timer::after_millis(50)).await {
                    Either::First(CanEvent::RequestShutdown) => {
                        force_shutdown(&mut led_g, &mut led_r, &mut led_y);
                        state = State::IDLE;
                    }
                    Either::First(_) => {} // unexpected events, ignore
                    Either::Second(_) => {
                        if button_r.is_high() {
                            normal_shutdown(&mut tx, &mut led_g, &mut led_r, &mut led_y).await;
                            state = State::IDLE;
                        }
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

#[repr(u8)]
enum Module {
    DriverInterface = 0b0100,
}

#[repr(u64)]
enum Instructions {
    // ForceShutdown = 0x10,  Cockpit → DriverInterface: force immediate shutdown to all PCBs (Cockpit should not send this instruction since no major fault should occur in the cockpit, but this instruction is defined as an example.)
    Start    = 0x11, // Cockpit → DriverInterface: inform that user wants to start the system
    Shutdown = 0x12, // Cockpit → DriverInterface: inform that user wants to shutdown the system
    RequestShutdown             = 0x40, // DriverInterface → Cockpit: requesting immediate shutdown
    ValidationForStartingState  = 0x41, // DriverInterface → Cockpit: ready to enter RUNNING
    ValidationForShutdownState  = 0x42, // DriverInterface → Cockpit: ready to return to IDLE
}
