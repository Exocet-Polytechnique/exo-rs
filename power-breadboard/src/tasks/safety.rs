use defmt::*;

use embassy_futures::select::{Either, select};
use embassy_stm32::gpio::{Input, Output};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::{self, Watch}};
use embassy_time::Timer;

use crate::tasks::{can::{BoatState, CURRENT_STATE, ERROR_CHANNEL, ErrorType, WARNING_CHANNEL, WarningType}, contactors::{CURRENT_CONTACTORS_STATE, ContactorsState}};

const TICK_DELAY_MS: u64 = 250;
const DMS_RESET_DELAY_MS: u64 = 10;

pub struct SafetyGpios {
    // Inputs
    pub alarm_status: Input<'static>,
    pub dms_fault_latch: Input<'static>,
    pub dms_status: Input<'static>,

    // Outputs
    pub alarm_force: Output<'static>,
    pub enable_dms: Output<'static>,
    pub reset_dms: Output<'static>,
}

#[derive(PartialEq, Clone, Copy)]
pub enum SafetyState {
    Idle,
    WaitingRemoval,
    WaitingInsertion,
    Ready,
    DmsFault,
    AlarmFault,
    WaitingContactorShutdown,
}

pub static CURRENT_SAFETY_STATE: Watch<CriticalSectionRawMutex, SafetyState, 1> = Watch::new();
type SafetyStateSender = watch::Sender<'static, CriticalSectionRawMutex, SafetyState, 1>;

async fn goto_state(gpios: &mut SafetyGpios, mut target: SafetyState, state_sender: &SafetyStateSender) {
    match target {
        SafetyState::Idle => {
            gpios.enable_dms.set_low();
        }
        SafetyState::WaitingRemoval => {
            if gpios.dms_status.is_high() {
                _ = WARNING_CHANNEL.try_send(WarningType::InsertDms);
                target = SafetyState::WaitingInsertion;
            } else {
                _ = WARNING_CHANNEL.try_send(WarningType::RemoveDms);
            }
        }
        SafetyState::WaitingInsertion => {
            _ = WARNING_CHANNEL.try_send(WarningType::InsertDms);
        }
        SafetyState::Ready => {
            gpios.enable_dms.set_high();
            gpios.reset_dms.set_high();
            Timer::after_millis(DMS_RESET_DELAY_MS).await;
            gpios.reset_dms.set_low();
        }
        SafetyState::DmsFault => {
            _ = ERROR_CHANNEL.try_send(ErrorType::DmsFault);
        }
        SafetyState::AlarmFault => {
            _ = ERROR_CHANNEL.try_send(ErrorType::IsolationFault);
        }
        _ => {}
    }

    state_sender.send(target);
}

async fn state_tick(gpios: &mut SafetyGpios, state_sender: &SafetyStateSender) {
    match CURRENT_SAFETY_STATE.try_get().unwrap() {
        SafetyState::WaitingRemoval => {
            if gpios.dms_status.is_high() {
                goto_state(gpios, SafetyState::WaitingInsertion, state_sender).await;
            }
        }
        SafetyState::WaitingInsertion => {
            if gpios.dms_status.is_low() {
                goto_state(gpios, SafetyState::Ready, state_sender).await;
            }
        }
        SafetyState::Ready => {
            // TODO: uncomment
            // if gpios.dms_status.is_high() {
            //     goto_state(gpios, SafetyState::DmsFault, state_sender).await;
            // } else if gpios.alarm_status.is_high() {
            //     goto_state(gpios, SafetyState::AlarmFault, state_sender).await;
            // }
        }
        SafetyState::WaitingContactorShutdown => {
            if let Some(contactors_state) = CURRENT_CONTACTORS_STATE.try_get() {
                if contactors_state == ContactorsState::Idle {
                    goto_state(gpios, SafetyState::Idle, state_sender).await;
                }
            }
        }
        _ => {}
    }
}

#[embassy_executor::task]
pub async fn safety_task(mut gpios: SafetyGpios) {
    let mut state_receiver = CURRENT_STATE.receiver().unwrap();

    let state_sender = CURRENT_SAFETY_STATE.sender();
    state_sender.send(SafetyState::Idle);

    loop {
        match select(Timer::after_millis(TICK_DELAY_MS), state_receiver.changed()).await {
            Either::First(_) => {
                state_tick(&mut gpios, &state_sender).await;
            }
            Either::Second(new_state) => {
                if new_state == BoatState::Startup {
                    goto_state(&mut gpios, SafetyState::WaitingRemoval, &state_sender).await;
                } else if new_state == BoatState::Shutdown {
                    goto_state(&mut gpios, SafetyState::WaitingContactorShutdown, &state_sender).await;
                }
            }
        }
    }
}
