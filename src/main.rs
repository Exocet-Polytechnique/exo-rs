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

// External events derived from DriverInterfaceHAT's LP_PCB05_P broadcast frame over CAN.
// These are sent to the cockpit task via CAN_CHANNEL.
// - ValidationForStartingState: CurrentState (MessageType=1) reports Started
// - ValidationForShutdownState: CurrentState (MessageType=1) reports Idle
// - ForceShutdown: Command (MessageType=2) addressed to us (or Broadcast) is ForceShutdown
#[derive(Clone, Copy)]
enum CanEvent {
    ValidationForStartingState,
    ValidationForShutdownState,
    ForceShutdown,
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

    // Configure CAN filter to accept only DriverInterfaceHAT's frames (HP_PCB05_E=271,
    // LP_PCB05_E=1295, LP_PCB05_P=1311) and reject all others. Only LP_PCB05_P is acted on
    // today; the error/warning frames are admitted for future use.
    // Mask 0x3EF covers every bit the three IDs share; the remaining two bits are
    // exactly what distinguishes them from each other, so no other message ID matches.
    configurator.properties().set_standard_filter(
        StandardFilterSlot::_0,
        StandardFilter {
            filter: FilterType::BitMask { filter: 271_u16, mask: 0x3EF },
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
                if id != dbc_gen::LpPcb05P::MESSAGE_ID {
                    continue;
                }
                let Ok(mut lp_p) = dbc_gen::LpPcb05P::try_from(envelope.frame.data()) else {
                    continue;
                };
                match lp_p.message_type() {
                    Ok(dbc_gen::LpPcb05PMessageType::M1(m1)) => match m1.current_state() {
                        dbc_gen::LpPcb05PCurrentState::Started => {
                            CAN_CHANNEL.send(CanEvent::ValidationForStartingState).await;
                        }
                        dbc_gen::LpPcb05PCurrentState::Idle => {
                            CAN_CHANNEL.send(CanEvent::ValidationForShutdownState).await;
                        }
                        _ => {}
                    },
                    Ok(dbc_gen::LpPcb05PMessageType::M2(m2)) => {
                        let target = m2.target_module();
                        if target != dbc_gen::LpPcb05PTargetModule::CockpitEcu
                            && target != dbc_gen::LpPcb05PTargetModule::Broadcast
                        {
                            continue;
                        }
                        if m2.command() == dbc_gen::LpPcb05PCommand::ForceShutdown {
                            CAN_CHANNEL.send(CanEvent::ForceShutdown).await;
                        }
                    }
                    _ => {}
                }
            }
            Err(_) => error!("CAN read error"),
        }
    }
}

/// Sends a Command addressed to `target` on our own LP_PCB01_P frame.
async fn send_command(
    tx: &mut can::CanTx<'static>,
    target: dbc_gen::LpPcb01PTargetModule,
    command: dbc_gen::LpPcb01PCommand,
) {
    let mut m2 = dbc_gen::LpPcb01PMessageTypeM2::new();
    if m2.set_target_module(target.into()).is_err() {
        return;
    }
    if m2.set_command(command.into()).is_err() {
        return;
    }
    if let Ok(mut lp_p) = dbc_gen::LpPcb01P::new(0) {
        if lp_p.set_m2(m2).is_ok() {
            if let Ok(f) = Frame::new_standard(dbc_gen::LpPcb01P::MESSAGE_ID as u16, lp_p.raw()) {
                tx.write(&f).await;
            }
        }
    }
}

/// Normal shutdown initiated by the cockpit button.
/// Sends a Shutdown Command to DriverInterfaceHAT and waits for ValidationForShutdownState before returning.
async fn normal_shutdown(
    tx: &mut can::CanTx<'static>,
    led_g: &mut Output<'_>,
    led_r: &mut Output<'_>,
    led_y: &mut Output<'_>,
) {
    send_command(
        tx,
        dbc_gen::LpPcb01PTargetModule::DriverInterfaceHat,
        dbc_gen::LpPcb01PCommand::Shutdown,
    )
    .await;
    info!("Shutdown Command sent to DriverInterfaceHAT");
    loop {
        match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
            Either::First(CanEvent::ValidationForShutdownState) => {
                info!("CockpitShutdown ACK received");
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
                    send_command(
                        &mut tx,
                        dbc_gen::LpPcb01PTargetModule::DriverInterfaceHat,
                        dbc_gen::LpPcb01PCommand::Start,
                    )
                    .await;
                    info!("Start Command sent to DriverInterfaceHAT");
                    state = State::STARTING;
                }

                Timer::after_millis(50).await;
            }

            State::STARTING => {
                match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
                    Either::First(CanEvent::ValidationForStartingState) => {
                        info!("CockpitStart ACK received, entering RUNNING");
                        led_g.set_low();
                        state = State::RUNNING;
                    }
                    Either::First(CanEvent::ForceShutdown) => {
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
                    Either::First(CanEvent::ForceShutdown) => {
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
