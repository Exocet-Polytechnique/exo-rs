//! CAN frame definitions for the `TelemetryBatteryECU` node, matching `exo-can/exo_can.dbc`.

use embassy_stm32::can::frame::Frame;

/// `HP_PCB03_E`: high-priority error frame.
const ERROR_FRAME_ID: u16 = 143;
/// `LP_PCB03_E`: low-priority warning frame.
const WARNING_FRAME_ID: u16 = 1167;
/// `LP_PCB03_D`: sensor data frame, multiplexed by [`Sensor`].
const DATA_FRAME_ID: u16 = 1199;
/// `LP_PCB03_P`: our own procedure/state frame. Only the CurrentState group (MessageType=1) is
/// used here; this board has no Start/Shutdown Command handling of its own.
const PROCEDURE_FRAME_ID: u16 = 1183;
/// `LP_PCB05_P`: DriverInterfaceHAT's procedure/state frame — listened to for its CurrentState
/// announcements (Starting/ShuttingDown) so we know when to confirm our own state.
pub const DASHBOARD_PROCEDURE_FRAME_ID: u16 = 1311;

#[repr(u8)]
enum Sensor {
    Battery = 0,
    Temperature = 1,
}

#[repr(u16)]
#[derive(Clone, Copy)]
pub enum ErrorType {
    BatteryFault = 0,
    Overtemperature = 1,
    #[allow(dead_code)]
    PowerSwitchFault = 2,
}

#[repr(u16)]
#[derive(Clone, Copy)]
pub enum WarningType {
    LowCharge = 0,
    TemperatureWarning = 1,
}

/// Our own CurrentState, sent on LP_PCB03_P (MessageType=1) to confirm a state change announced
/// by the dashboard. Values per the dbc's shared CurrentState table (Idle/Startup/Running/Shutdown).
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum CurrentState {
    Idle = 0,
    Running = 2,
}

/// DriverInterfaceHAT's CurrentState, decoded from its LP_PCB05_P (MessageType=1). Uses its own
/// value table (Idle/Starting/Started/ShuttingDown), distinct from every other PCB's CurrentState.
#[derive(Clone, Copy, PartialEq)]
pub enum DashboardState {
    Idle,
    Starting,
    Started,
    ShuttingDown,
}

const BATT_CURRENT_SCALE_A: f32 = 0.001953125;
const BATT_VOLTAGE_SCALE_V: f32 = 0.001953125;
const BATT_POWER_SCALE_W: f32 = 0.0625;
const TEMPERATURE_SCALE_C: f32 = 0.0625;

pub fn error_frame(error: ErrorType) -> Frame {
    Frame::new_standard(ERROR_FRAME_ID, &(error as u16).to_le_bytes()).unwrap()
}

pub fn warning_frame(warning: WarningType) -> Frame {
    Frame::new_standard(WARNING_FRAME_ID, &(warning as u16).to_le_bytes()).unwrap()
}

/// Build the `Sensor = BATTERY` variant of the sensor data frame: state of charge, current,
/// voltage and (derived) power.
pub fn battery_data_frame(soc_percent: f32, current_a: f32, voltage_v: f32) -> Frame {
    let power_w = current_a * voltage_v;

    let soc_raw = ((soc_percent / 100.0) * 255.0).clamp(0.0, 255.0) as u8;
    let current_raw = (current_a / BATT_CURRENT_SCALE_A).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    let voltage_raw = (voltage_v / BATT_VOLTAGE_SCALE_V).clamp(0.0, u16::MAX as f32) as u16;
    let power_raw = (power_w / BATT_POWER_SCALE_W).clamp(-8192.0, 8191.0) as i16;

    let mut data = [0u8; 8];
    data[0] = Sensor::Battery as u8;
    data[1] = soc_raw;
    data[2..4].copy_from_slice(&current_raw.to_le_bytes());
    data[4..6].copy_from_slice(&voltage_raw.to_le_bytes());
    data[6..8].copy_from_slice(&((power_raw as u16) & 0x3FFF).to_le_bytes());

    Frame::new_standard(DATA_FRAME_ID, &data).unwrap()
}

/// Build the `Sensor = TEMPERATURE` variant of the sensor data frame.
pub fn temperature_data_frame(temperature_c: f32) -> Frame {
    let temp_raw = (temperature_c / TEMPERATURE_SCALE_C).clamp(-2048.0, 2047.0) as i16;

    let mut data = [0u8; 8];
    data[0] = Sensor::Temperature as u8;
    data[1..3].copy_from_slice(&((temp_raw as u16) & 0x0FFF).to_le_bytes());

    Frame::new_standard(DATA_FRAME_ID, &data).unwrap()
}

/// Build our own LP_PCB03_P CurrentState confirmation. MessageType (bits 0-1) = 1 (CurrentState
/// group), CurrentState (bits 2-7) = `state` — both fit in a single byte, byte 1 unused.
pub fn state_frame(state: CurrentState) -> Frame {
    let byte0 = ((state as u8) << 2) | 0b01;
    Frame::new_standard(PROCEDURE_FRAME_ID, &[byte0, 0]).unwrap()
}

/// Decode an incoming LP_PCB05_P payload's CurrentState. Returns `None` if the frame isn't
/// currently announcing CurrentState (MessageType != 1, e.g. it's a ProcedureStatus or Command).
pub fn decode_dashboard_state(data: &[u8]) -> Option<DashboardState> {
    let byte0 = *data.first()?;
    if byte0 & 0b11 != 1 {
        return None;
    }
    match byte0 >> 2 {
        0 => Some(DashboardState::Idle),
        1 => Some(DashboardState::Starting),
        2 => Some(DashboardState::Started),
        3 => Some(DashboardState::ShuttingDown),
        _ => None,
    }
}
