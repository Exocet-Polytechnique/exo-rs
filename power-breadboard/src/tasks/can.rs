use defmt::*;
use embassy_futures::select::{Either4, select4};
use embassy_stm32::can::{Can, Frame};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, watch::{self, Watch}};
use embassy_time::Timer;

use crate::dbc_gen;

pub enum CanData {
    AuxBatteryTemperature(f32),
    ContactorBitset(u8),
    MpptOutputVoltage(f32),
    MpptOutputCurrent(f32),
    MpptCharge(u8),
    MpptInputVoltage(f32),
    MpptInputCurrent(f32),
    ImdResistance(f32),
    ImdStatus(bool),
    Temperature(u16),
}

#[derive(Debug)]
pub enum ErrorType {
    AuxBattTempSensorMissing = 0x4000,
    AuxBattTempSensorDisconnected = 0x4001,
    AuxBattTempTooHigh = 0x4002,
    DmsFault = 0x4003,
    IsolationFault = 0x4004,
}

#[derive(Debug)]
pub enum WarningType {
    AuxBattTempHigh = 0x4300,
    RemoveDms = 0x4301,
    InsertDms = 0x4302,
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
    let id = match frame.id() {
        embedded_can::Id::Standard(id) => id.as_raw() as u32,
        embedded_can::Id::Extended(_) => return,
    };
    let data = frame.data();

    match id {
        // High priority errors from HighPowerECU (ID 207)
        dbc_gen::HpPcb04E::MESSAGE_ID => {
            if let Ok(msg) = dbc_gen::HpPcb04E::try_from(data) {
                let error = match msg.error_type_raw() {
                    0 => ErrorType::AuxBattTempSensorMissing,
                    1 => ErrorType::AuxBattTempSensorDisconnected,
                    2 => ErrorType::AuxBattTempTooHigh,
                    3 => ErrorType::DmsFault,
                    4 => ErrorType::IsolationFault,
                    _ => return,
                };
                ERROR_CHANNEL.send(error).await;
            }
        }

        // Low priority warnings from HighPowerECU (ID 1231)
        dbc_gen::LpPcb04E::MESSAGE_ID => {
            if let Ok(msg) = dbc_gen::LpPcb04E::try_from(data) {
                let warning = match msg.warning_type_raw() {
                    0 => WarningType::AuxBattTempHigh,
                    1 => WarningType::RemoveDms,
                    2 => WarningType::InsertDms,
                    _ => return,
                };
                WARNING_CHANNEL.send(warning).await;
            }
        }

        // Procedure/command frame from HighPowerECU (ID 1247)
        dbc_gen::LpPcb04P::MESSAGE_ID => {
            if let Ok(mut msg) = dbc_gen::LpPcb04P::try_from(data) {
                if msg.message_type_raw() == 1 {
                    if let Ok(dbc_gen::LpPcb04PMessageType::M1(m1)) = msg.message_type() {
                        let state = match m1.current_state_raw() {
                            0 => BoatState::Idle,
                            1 => BoatState::Startup,
                            2 => BoatState::Running,
                            3 => BoatState::Shutdown,
                            _ => return,
                        };
                        CURRENT_STATE.sender().send(state);
                    }
                }
            }
        }

        // Sensor data from HighPowerECU (ID 1263)
        dbc_gen::LpPcb04D::MESSAGE_ID => {
            if let Ok(mut msg) = dbc_gen::LpPcb04D::try_from(data) {
                match msg.sensor_raw() {
                    0 => {
                        if let Ok(dbc_gen::LpPcb04DSensor::M0(m0)) = msg.sensor() {
                            DATA_CHANNEL.send(CanData::AuxBatteryTemperature(
                                m0.aux_battery_temperature_raw()
                            )).await;
                        }
                    }
                    // Fuel cell data
                    1 => {
                    }
                    2 => {
                    }
                    3 => {
                    }
                    4 => {
                        if let Ok(dbc_gen::LpPcb04DSensor::M4(m4)) = msg.sensor() {
                            DATA_CHANNEL.send(CanData::ContactorBitset(m4.contactor_bitset_raw())).await;
                        }
                    }
                    5 => {
                        if let Ok(dbc_gen::LpPcb04DSensor::M5(m5)) = msg.sensor() {
                            DATA_CHANNEL.send(CanData::MpptOutputVoltage(m5.mppt_output_voltage_raw())).await;
                            DATA_CHANNEL.send(CanData::MpptOutputCurrent(m5.mppt_output_current_raw())).await;
                            DATA_CHANNEL.send(CanData::MpptCharge(m5.mppt_charge_raw())).await;
                        }
                    }
                    6 => {
                        if let Ok(dbc_gen::LpPcb04DSensor::M6(m6)) = msg.sensor() {
                            DATA_CHANNEL.send(CanData::MpptInputVoltage(m6.mppt_input_voltage_raw())).await;
                            DATA_CHANNEL.send(CanData::MpptInputCurrent(m6.mppt_input_current_raw())).await;
                        }
                    }
                    7 => {
                        if let Ok(dbc_gen::LpPcb04DSensor::M7(m7)) = msg.sensor() {
                            DATA_CHANNEL.send(CanData::ImdResistance(m7.imd_resistance_raw())).await;
                            DATA_CHANNEL.send(CanData::ImdStatus(m7.imd_status_raw())).await;
                        }
                    }
                    _ => {}
                }
            }
        }

        _ => {}
    }
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
        CanData::AuxBatteryTemperature(_) => {}
        CanData::ContactorBitset(_) => {}
        CanData::MpptOutputVoltage(_) => {}
        CanData::MpptOutputCurrent(_) => {}
        CanData::MpptCharge(_) => {}
        CanData::MpptInputVoltage(_) => {}
        CanData::MpptInputCurrent(_) => {}
        CanData::ImdResistance(_) => {}
        CanData::ImdStatus(_) => {}
    }
}

// async fn send_error(error: ErrorType) {
//     match error {
//         ErrorType::Err(e) => {
//             info!("Error: {}", e as u16)
//         }
//     }
// }

// async fn send_warning(error: WarningType) {
//     match error {
//         WarningType::Warn(w) => {
//             info!("Warning: {}", w as u16)
//         }
//     }
// }

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
                send_data(can_bus, data).await;
            }
            Either4::Third(error) => {
                //send_error(ErrorType::Err(error)).await;
            }
            Either4::Fourth(warning) => {
                //send_warning(WarningType::Warn(warning)).await;
            }
        }
    }
}
