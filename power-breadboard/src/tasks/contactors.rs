use embassy_stm32::gpio::Output;
use embassy_time::Timer;

const TICK_DELAY_MS: u64 = 100;

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

#[derive(PartialEq)]
pub enum ContactorsState {
    Idle,
}

#[embassy_executor::task]
pub async fn contactors_task(mut ouputs: ContactorOutputs) {
    loop {
        Timer::after_millis(TICK_DELAY_MS).await;
    }
}
