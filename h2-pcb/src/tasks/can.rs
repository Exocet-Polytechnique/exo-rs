use defmt::*;
use embassy_futures::select::{Either4, select4};
use embassy_stm32::can::{Can, Frame};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, watch::{self, Watch}};
use embassy_time::Timer;

pub enum CanData {
    Temperature(u16),
    Pressure(f32),
    Actuators(u8),
}

#[derive(Debug)]
pub enum ErrorType {
    PlaqueTempSensorMissing = 0,
    PlaqueTempSensorDisconnected = 1,
    TankTempSensorMissing = 2,
    TankTempSensorDisconnected = 3,
    PlaqueTempTooHigh = 4,
    TankTempTooHigh = 5,
    Manometer1PressureTooHigh = 6,
    Manometer2PressureTooHigh = 7,
    Manometer1PressureTooLow = 8,
    Manometer2PressureTooLow = 9,
    Actuator1InvalidPosition = 10,
    Actuator2InvalidPosition = 11,
    Actuator3InvalidPosition = 12
}

pub enum WarningType {
    PlaqueTempHigh = 0,
    TankTempHigh = 1,
    Manometer1PressureHigh = 2,
    Manometer2PressureHigh = 3,
    Manometer1PressureLow = 4,
    Manometer2PressureLow = 5,
}

#[derive(Clone, PartialEq)]
pub enum BoatState {
    Idle = 0,
    Startup = 1,
    Running = 2,
    Shutdown = 3,
}

// Input signals
pub static DATA_CHANNEL: Channel<CriticalSectionRawMutex, CanData, 8> = Channel::new();
pub static ERROR_CHANNEL: Channel<CriticalSectionRawMutex, ErrorType, 16> = Channel::new();
pub static WARNING_CHANNEL: Channel<CriticalSectionRawMutex, WarningType, 16> = Channel::new();

// Output signals
pub static CURRENT_STATE: Watch<CriticalSectionRawMutex, BoatState, 2> = Watch::new();

pub type StateReceiver = watch::Receiver<'static, CriticalSectionRawMutex, BoatState, 2>;

async fn receive_frame(frame: Frame) {
    // match frame.data()[0] {
    //
    // }
}

async fn send_data(data: CanData) {
}

async fn send_error(error: ErrorType) {
}

async fn send_warning(error: WarningType) {
}

#[embassy_executor::task]
pub async fn can_task(mut can_bus: Can<'static>) {
    let state_sender = CURRENT_STATE.sender();
    state_sender.send(BoatState::Idle);
    Timer::after_millis(1000).await;
    state_sender.send(BoatState::Startup);

    loop {
        match select4(can_bus.read(), DATA_CHANNEL.receive(), ERROR_CHANNEL.receive(), WARNING_CHANNEL.receive()).await {
            Either4::First(recv_result) => {
                // Ignore errors here
                if let Ok(envelope) = recv_result {
                    receive_frame(envelope.frame).await;
                }
            }
            Either4::Second(data) => {
                send_data(data).await;
            }
            Either4::Third(error) => {
                send_error(error).await;
            }
            Either4::Fourth(warning) => {
                send_warning(warning).await;
            }
        }
    }
}