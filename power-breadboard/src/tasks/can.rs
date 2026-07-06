use embassy_futures::select::{Either4, select4};
use embassy_stm32::can::{Can, Frame};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, watch::{self, Watch}};

pub enum CanData {
    Temperature(u16)
}

#[derive(Debug)]
pub enum ErrorType {
    AuxBattTempSensorMissing = 0,
    AuxBattTempSensorDisconnected = 1,
    AuxBattTempTooHigh = 2,
    DmsFault = 3,
    IsolationFault = 4,
}

pub enum WarningType {
    AuxBattTempHigh = 0,
    RemoveDms = 1,
    InsertDms = 2,
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
}

async fn send_data(data: CanData) {
}


enum ErrorOrWarning {
    Err(ErrorType),
    Warn(WarningType),
}

async fn send_error(error: ErrorOrWarning) {
}

#[embassy_executor::task]
pub async fn can_task(mut can_bus: Can<'static>) {
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
                send_error(ErrorOrWarning::Err(error)).await;
            }
            Either4::Fourth(warning) => {
                send_error(ErrorOrWarning::Warn(warning)).await;
            }
        }
    }
}
