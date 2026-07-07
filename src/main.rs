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
use embassy_time::{Duration, Instant, Timer};
use {defmt_rtt as _, panic_probe as _};

pub mod dbc_gen {
    include!(concat!(env!("OUT_DIR"), "/dbc_gen.rs"));
}

static mut ERROR_FLAG: bool = false;

static CAN_CHANNEL: Channel<CriticalSectionRawMutex, CanEvent, 4> = Channel::new();

// If DriverInterfaceHAT's LP_PCB05_P heartbeat (MessageType=1, CurrentState) hasn't been seen
// for this long while STARTING/RUNNING, treat it as a dead dashboard and fault out.
const HEARTBEAT_TIMEOUT_MS: u64 = 8000;

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

// External events derived from DriverInterfaceHAT's frames over CAN, sent to the cockpit task
// via CAN_CHANNEL.
// - DashboardState: LP_PCB05_P CurrentState (MessageType=1) heartbeat — sent on every occurrence,
//   regardless of value, so the cockpit task can use it both to detect Started/Idle and to know
//   the dashboard is still alive.
// - ForceShutdown: LP_PCB05_P Command (MessageType=2) addressed to us (or Broadcast) is ForceShutdown
// - CriticalError: HP_PCB05_E (any error code)
#[derive(Clone, Copy)]
enum CanEvent {
    DashboardState(dbc_gen::LpPcb05PCurrentState),
    ForceShutdown,
    CriticalError,
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
                if id == dbc_gen::LpPcb05P::MESSAGE_ID {
                    let Ok(mut lp_p) = dbc_gen::LpPcb05P::try_from(envelope.frame.data()) else {
                        continue;
                    };
                    match lp_p.message_type() {
                        Ok(dbc_gen::LpPcb05PMessageType::M1(m1)) => {
                            CAN_CHANNEL.send(CanEvent::DashboardState(m1.current_state())).await;
                        }
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
                } else if id == dbc_gen::HpPcb05E::MESSAGE_ID {
                    if dbc_gen::HpPcb05E::try_from(envelope.frame.data()).is_ok() {
                        CAN_CHANNEL.send(CanEvent::CriticalError).await;
                    }
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
/// Sends a Shutdown Command to DriverInterfaceHAT and waits for its CurrentState heartbeat to
/// report Idle before returning.
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
            Either::First(CanEvent::DashboardState(dbc_gen::LpPcb05PCurrentState::Idle)) => {
                info!("CockpitShutdown ACK received (dashboard reports Idle)");
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
    // Last time we heard DriverInterfaceHAT's LP_PCB05_P heartbeat. Only enforced as a timeout
    // in STARTING/RUNNING, where the system actively depends on the dashboard being alive.
    let mut last_heartbeat = Instant::now();
    loop {
        if unsafe { ERROR_FLAG } {
            state = State::FAULT;
        }

        match state {
            State::IDLE => {
                led_r.set_high();
                led_g.set_low();
                led_y.set_low();

                // Drain CAN_CHANNEL even while idle so the heartbeat (sent continuously by the
                // dashboard) never fills the channel and stalls can_reader.
                match select(CAN_CHANNEL.receive(), Timer::after_millis(50)).await {
                    Either::First(CanEvent::DashboardState(_)) => {
                        last_heartbeat = Instant::now();
                    }
                    Either::First(CanEvent::CriticalError) => {
                        unsafe { ERROR_FLAG = true; }
                    }
                    Either::First(_) => {}
                    Either::Second(_) => {
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
                    }
                }
            }

            State::STARTING => {
                match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
                    Either::First(CanEvent::DashboardState(s)) => {
                        last_heartbeat = Instant::now();
                        if s == dbc_gen::LpPcb05PCurrentState::Started {
                            info!("Dashboard reports Started, entering RUNNING");
                            led_g.set_low();
                            state = State::RUNNING;
                        }
                    }
                    Either::First(CanEvent::ForceShutdown) => {
                        force_shutdown(&mut led_g, &mut led_r, &mut led_y);
                        state = State::IDLE;
                    }
                    Either::First(CanEvent::CriticalError) => {
                        unsafe { ERROR_FLAG = true; }
                    }
                    Either::Second(_) => {
                        if last_heartbeat.elapsed() > Duration::from_millis(HEARTBEAT_TIMEOUT_MS) {
                            error!("Dashboard heartbeat timed out while STARTING");
                            unsafe { ERROR_FLAG = true; }
                        } else {
                            // Still waiting: blink green
                            led_g.toggle();
                            led_r.set_low();
                            led_y.set_low();
                        }
                    }
                }
            }

            State::RUNNING => {
                led_g.set_high();
                led_r.set_low();
                led_y.set_low();

                match select(CAN_CHANNEL.receive(), Timer::after_millis(50)).await {
                    Either::First(CanEvent::DashboardState(_)) => {
                        last_heartbeat = Instant::now();
                    }
                    Either::First(CanEvent::ForceShutdown) => {
                        force_shutdown(&mut led_g, &mut led_r, &mut led_y);
                        state = State::IDLE;
                    }
                    Either::First(CanEvent::CriticalError) => {
                        unsafe { ERROR_FLAG = true; }
                    }
                    Either::Second(_) => {
                        if last_heartbeat.elapsed() > Duration::from_millis(HEARTBEAT_TIMEOUT_MS) {
                            error!("Dashboard heartbeat timed out while RUNNING");
                            unsafe { ERROR_FLAG = true; }
                        } else if button_r.is_high() {
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
                // Keep draining CAN_CHANNEL so a live dashboard doesn't back up while faulted.
                let _ = select(CAN_CHANNEL.receive(), Timer::after_millis(10)).await;
            }
        }
    }
}
