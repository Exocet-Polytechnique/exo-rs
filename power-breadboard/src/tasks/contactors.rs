use embassy_stm32::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};
use embassy_time::Timer;

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

#[embassy_executor::task]
pub async fn contactors_task(mut ouputs: ContactorOutputs) {
    // let state_sender = CURRENT_CONTACTORS_STATE.sender();
    // state_sender.send(ContactorsState::Idle);

    loop {
        Timer::after_millis(TICK_DELAY_MS).await;
    }
}
