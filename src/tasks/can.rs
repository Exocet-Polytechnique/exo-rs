use defmt::*;
use embassy_stm32::can::{self, Frame};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

use crate::dbc_gen;

pub static CAN_CHANNEL: Channel<CriticalSectionRawMutex, CanEvent, 4> = Channel::new();

// External events derived from DriverInterfaceHAT's frames over CAN, sent to the cockpit task
// via CAN_CHANNEL.
// - DashboardState: LP_PCB05_P CurrentState (MessageType=1) — sent once whenever the boat's
//   state actually changes (not periodic), so the cockpit task can detect Started/Idle.
// - ForceShutdown: LP_PCB05_P Command (MessageType=2) addressed to us (or Broadcast) is ForceShutdown
// - CriticalError: HP_PCB05_E (any error code)
#[derive(Clone, Copy)]
pub enum CanEvent {
    DashboardState(dbc_gen::LpPcb05PCurrentState),
    ForceShutdown,
    CriticalError,
}

/// Reads all incoming CAN frames and forwards parsed events to CAN_CHANNEL.
#[embassy_executor::task]
pub async fn can_reader(mut rx: can::CanRx<'static>) {
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
pub async fn send_command(
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

/// Confirms our own CurrentState on our own LP_PCB01_P frame — sent to DriverInterfaceHAT in
/// response to its state-change announcement (see LpPcb05PCurrentState in can_reader).
pub async fn send_state(tx: &mut can::CanTx<'static>, state: dbc_gen::LpPcb01PCurrentState) {
    let mut m1 = dbc_gen::LpPcb01PMessageTypeM1::new();
    if m1.set_current_state(state.into()).is_err() {
        return;
    }
    if let Ok(mut lp_p) = dbc_gen::LpPcb01P::new(1) {
        if lp_p.set_m1(m1).is_ok() {
            if let Ok(f) = Frame::new_standard(dbc_gen::LpPcb01P::MESSAGE_ID as u16, lp_p.raw()) {
                tx.write(&f).await;
            }
        }
    }
}

// Raw ErrorType codes we send on our own HP_PCB01_E. 0/1 (CAN_BUS_FAULT/HARDWARE_FAULT) are
// reserved by the dbc's VAL_ table; CAN_TIMEOUT isn't in it yet (it's currently misfiled there
// as a WarningType on LP_PCB01_E, which we don't send — per the spec doc this is critical).
pub mod error {
    pub const CAN_TIMEOUT: u16 = 2;
}

/// Reports our own detected fault via HP_PCB01_E — critical, since a command with no
/// confirmation means we can no longer trust we've actually changed the boat's state.
pub async fn send_error(tx: &mut can::CanTx<'static>, error_type: u16) {
    if let Ok(frame) = dbc_gen::HpPcb01E::new(error_type) {
        if let Ok(f) = Frame::new_standard(dbc_gen::HpPcb01E::MESSAGE_ID as u16, frame.raw()) {
            tx.write(&f).await;
        }
    }
}
