use defmt::*;
use embassy_futures::select::{Either4, select4};
use embassy_stm32::can::{Can, Frame};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, watch::{self, Watch}};
use embassy_time::Timer;

use crate::dbc_gen;

pub enum CanData {
    Temperature(u16)
}

#[derive(Debug)]
pub enum ErrorType {
    AuxBattTempSensorMissing = 0x300,
    AuxBattTempSensorDisconnected = 0x301,
    AuxBattTempTooHigh = 0x302,
    DmsFault = 0x303,
    IsolationFault = 0x304,
}

#[derive(Debug)]
pub enum WarningType {
    AuxBattTempHigh = 0x8300,
    RemoveDms = 0x8301,
    InsertDms = 0x8302,
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

async fn send_data(can_bus: &mut Can<'static>, data: CanData) {
    match data {
        CanData::Temperature(raw) => {
            let mut m0 = dbc_gen::LpPcb04DSensorM0::new();
            // `raw` is already to_fixed16(celsius, 4) == celsius * 16, which is exactly the
            // AuxBatteryTemperature signal's raw encoding (factor 0.0625 == 1/16). The generated
            // setter takes physical Celsius and re-applies that factor, so convert back first.
            let celsius = (raw as i16) as f32 / 16.0;
            if m0.set_aux_battery_temperature(celsius).is_err() {
                return;
            }
            if let Ok(mut lp_d) = dbc_gen::LpPcb04D::new(0) {
                if lp_d.set_m0(m0).is_ok() {
                    if let Ok(f) = Frame::new_standard(dbc_gen::LpPcb04D::MESSAGE_ID as u16, lp_d.raw()) {
                        can_bus.write(&f).await;
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
enum ErrorOrWarning {
    Err(ErrorType),
    Warn(WarningType),
}

async fn send_error(error: ErrorOrWarning) {
    match error {
        ErrorOrWarning::Err(e) => {
            info!("Error: {}", e as u16)
        }
        ErrorOrWarning::Warn(w) => {
            info!("Warning: {}", w as u16)
        }
    }
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
                send_data(&mut can_bus, data).await;
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
