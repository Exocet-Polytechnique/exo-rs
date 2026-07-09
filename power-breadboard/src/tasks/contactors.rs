use embassy_futures::select::{Either, select};
use embassy_stm32::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::{self, Watch}};
use embassy_time::Timer;

use crate::tasks::{can::{BoatState, CURRENT_STATE}, safety::{CURRENT_SAFETY_STATE, SafetyState}};

const TICK_DELAY_MS: u64 = 250;

pub struct ContactorOutputs {
    pub bop: Output<'static>,
    pub storage_battery: Output<'static>,
    pub mppt_stage_1: Output<'static>,
    pub mppt_stage_2: Output<'static>,
    pub n_motor_stage_1: Output<'static>,
    pub n_motor_stage_2: Output<'static>,
    pub n_braking_resistor: Output<'static>,
    pub n_auxiliary_battery: Output<'static>,
}

#[derive(PartialEq, Clone)]
pub enum ContactorsState {
    Idle,
    WaitingForDmsTest,
    MpptInitialization,
    FuelCellInitialization,
    MpptPrecharge1, // 1. precharge resistor
    MpptPrecharge2, // 2. full power
    MotorPrecharge1, // 1. precharge resistor
    MotorPrecharge2, // 2. braking resistor
    MotorPrecharge3, // 3. full power
    Ready,
    MotorPostcharge1, // 1. precharge resistor
    MotorPostcharge2, // 2. cut full power
    MotorPostcharge3, // 3. braking resistor
    FuelCellShutdown,
    MpptPostcharge1, // 1. precharge resistor
    MpptPostcharge2, // 2. full power
    MpptShutdown,
}

pub static CURRENT_CONTACTORS_STATE: Watch<CriticalSectionRawMutex, ContactorsState, 1> = Watch::new();
type ContactorStateSender = watch::Sender<'static, CriticalSectionRawMutex, ContactorsState, 1>;

static mut CURRENT_STATE_TICK_COUNT: u32 = 0;

async fn goto_state(outputs: &mut ContactorOutputs, target: ContactorsState, state_sender: &ContactorStateSender) {
    match target {
        ContactorsState::Idle => {
            outputs.bop.set_low();
            outputs.storage_battery.set_low();
            outputs.mppt_stage_1.set_low();
            outputs.mppt_stage_2.set_low();
            outputs.n_motor_stage_1.set_high();
            outputs.n_motor_stage_2.set_high();
            outputs.n_braking_resistor.set_high();
            outputs.n_auxiliary_battery.set_high();
        }
        ContactorsState::WaitingForDmsTest => {}
        ContactorsState::MpptInitialization => {
            outputs.n_auxiliary_battery.set_low();
        }
        ContactorsState::FuelCellInitialization => {
            outputs.bop.set_high();
        }
        ContactorsState::MpptPrecharge1 => {
            outputs.mppt_stage_1.set_high();
            outputs.storage_battery.set_high();
        }
        ContactorsState::MpptPrecharge2 => {
            outputs.mppt_stage_2.set_high();
        }
        ContactorsState::MotorPrecharge1 => {
            outputs.n_motor_stage_1.set_low();
        }
        ContactorsState::MotorPrecharge2 => {
            outputs.n_braking_resistor.set_low();
        }
        ContactorsState::MotorPrecharge3 => {
            outputs.n_motor_stage_2.set_low();
        }
        ContactorsState::MotorPostcharge1 => {
            outputs.n_motor_stage_1.set_low();
        }
        ContactorsState::MotorPostcharge2 => {
            outputs.n_motor_stage_2.set_high();
        }
        ContactorsState::MotorPostcharge3 => {
            outputs.n_braking_resistor.set_high();
        }
        ContactorsState::FuelCellShutdown => {
            outputs.bop.set_low();
        }
        ContactorsState::MpptPostcharge1 => {
            outputs.mppt_stage_1.set_high();
        }
        ContactorsState::MpptPostcharge2 => {
            outputs.mppt_stage_2.set_low();
        }
        ContactorsState::MpptShutdown => {
            outputs.n_auxiliary_battery.set_high();
        }
        ContactorsState::Ready => {}
    }

    unsafe { CURRENT_STATE_TICK_COUNT = 0 };

    state_sender.send(target);
}

async fn state_tick(outputs: &mut ContactorOutputs, state_sender: &ContactorStateSender)
{
    unsafe { CURRENT_STATE_TICK_COUNT += 1 };

    match CURRENT_CONTACTORS_STATE.try_get().unwrap() {
        ContactorsState::WaitingForDmsTest => {
            if let Some(safety_state) = CURRENT_SAFETY_STATE.try_get() {
                if safety_state == SafetyState::Ready {
                    goto_state(outputs, ContactorsState::MpptInitialization, state_sender).await;
                }
            }
        }
        ContactorsState::MpptInitialization => {
            // NOTE: reduce delay since we don't use the MPPT. Also there is no need to communicate
            // with it at the moment.
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                goto_state(outputs, ContactorsState::FuelCellInitialization, state_sender).await;
            }
        }
        ContactorsState::FuelCellInitialization => {
            // NOTE: we won't really use the fuel cell so we can just continue to the next state
            goto_state(outputs, ContactorsState::MpptPrecharge1, state_sender).await;
        }
        ContactorsState::MpptPrecharge1 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 30) } {
                goto_state(outputs, ContactorsState::MpptPrecharge2, state_sender).await;
            }
        }
        ContactorsState::MpptPrecharge2 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                outputs.mppt_stage_1.set_low();
                goto_state(outputs, ContactorsState::MotorPrecharge1, state_sender).await;
            }
        }
        ContactorsState::MotorPrecharge1 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 30) } {
                goto_state(outputs, ContactorsState::MotorPostcharge2, state_sender).await;
            }
        }
        ContactorsState::MotorPrecharge2 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                goto_state(outputs, ContactorsState::MotorPostcharge3, state_sender).await;
            }
        }
        ContactorsState::MotorPrecharge3 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                outputs.n_motor_stage_1.set_high();
                goto_state(outputs, ContactorsState::Ready, state_sender).await;
            }
        }
        ContactorsState::MotorPostcharge1 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                goto_state(outputs, ContactorsState::MotorPostcharge2, state_sender).await;
            }
        }
        ContactorsState::MotorPostcharge2 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                goto_state(outputs, ContactorsState::MotorPostcharge3, state_sender).await;
            }
        }
        ContactorsState::MotorPostcharge3 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                outputs.n_motor_stage_1.set_high();
                goto_state(outputs, ContactorsState::FuelCellShutdown, state_sender).await;
            }
        }
        ContactorsState::FuelCellShutdown => {
            goto_state(outputs, ContactorsState::MpptPostcharge1, state_sender).await;
        }
        ContactorsState::MpptPostcharge1 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                goto_state(outputs, ContactorsState::MpptPostcharge2, state_sender).await;
            }
        }
        ContactorsState::MpptPostcharge2 => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                outputs.mppt_stage_1.set_low();
                outputs.storage_battery.set_low();
                goto_state(outputs, ContactorsState::MpptShutdown, state_sender).await;
            }
        }
        ContactorsState::MpptShutdown => {
            if unsafe { CURRENT_STATE_TICK_COUNT >= (4 * 2) } {
                goto_state(outputs, ContactorsState::Idle, state_sender).await;
            }
        }
        ContactorsState::Idle => {}
        ContactorsState::Ready => {}
    }
}

#[embassy_executor::task]
pub async fn contactors_task(mut outputs: ContactorOutputs) {
    let mut state_receiver = CURRENT_STATE.receiver().unwrap();

    let state_sender = CURRENT_CONTACTORS_STATE.sender();
    state_sender.send(ContactorsState::Idle);

    loop {
        match select(Timer::after_millis(TICK_DELAY_MS), state_receiver.changed()).await {
            Either::First(_) => {
                state_tick(&mut outputs, &state_sender).await;
            }
            Either::Second(new_state) => {
                if new_state == BoatState::Startup {
                    if CURRENT_CONTACTORS_STATE.try_get().unwrap() == ContactorsState::Idle {
                        goto_state(&mut outputs, ContactorsState::WaitingForDmsTest, &state_sender).await;
                    }
                } else if new_state == BoatState::Shutdown {
                }
            }
        }
    }
}
