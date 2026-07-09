//! CAN frame definitions for the `TelemetryBatteryECU` node, built from the generated
//! `exo-can/exo_can.dbc` types (see `crate::dbc_gen`) rather than hand-packed bytes.

use embassy_stm32::can::frame::Frame;

use crate::dbc_gen;

/// `LP_PCB05_P`: DriverInterfaceHAT's procedure/state frame — listened to for its CurrentState
/// announcements (Starting/ShuttingDown) so we know when to confirm our own state.
pub const DASHBOARD_PROCEDURE_FRAME_ID: u16 = dbc_gen::LpPcb05P::MESSAGE_ID as u16;

#[derive(Clone, Copy)]
pub enum ErrorType {
    BatteryFault,
    Overtemperature,
    #[allow(dead_code)]
    PowerSwitchFault,
}

#[derive(Clone, Copy)]
pub enum WarningType {
    LowCharge,
    TemperatureWarning,
}

/// Our own CurrentState, sent on LP_PCB03_P (MessageType=1) to confirm a state change announced
/// by the dashboard. Values per the dbc's shared CurrentState table (Idle/Startup/Running/Shutdown).
#[derive(Clone, Copy)]
pub enum CurrentState {
    Idle,
    Running,
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

// The dbc's VAL_ tables for HP_PCB03_E/LP_PCB03_E use small sequential values (0,1,2 / 0,1),
// but the dashboard (exo-server's dashboard_state.rs::error_title) has moved to its own
// 0x{PCB}{seq} convention and no longer looks those up — it expects 0x3000-range codes from
// TelemetryBattery specifically. Sending the dbc's raw values here would silently show up as
// "Erreur inconnue" on the dashboard. Must be kept in sync with error_title's match table.
pub fn error_frame(error: ErrorType) -> Frame {
    let code: u16 = match error {
        ErrorType::BatteryFault => 0x3000,
        ErrorType::Overtemperature => 0x3001,
        ErrorType::PowerSwitchFault => 0x3002,
    };
    // CanError doesn't derive Debug in this build, so unwrap() (which needs it) can't be used
    // directly on these Results; .ok().unwrap() sidesteps that (Option::unwrap has no such bound)
    // — fine since these are fixed, always-valid inputs; the error path is unreachable in practice.
    let frame = dbc_gen::HpPcb03E::new(code).ok().unwrap();
    Frame::new_standard(dbc_gen::HpPcb03E::MESSAGE_ID as u16, frame.raw()).ok().unwrap()
}

pub fn warning_frame(warning: WarningType) -> Frame {
    let code: u16 = match warning {
        WarningType::LowCharge => 0x3300,
        WarningType::TemperatureWarning => 0x3301,
    };
    let frame = dbc_gen::LpPcb03E::new(code).ok().unwrap();
    Frame::new_standard(dbc_gen::LpPcb03E::MESSAGE_ID as u16, frame.raw()).ok().unwrap()
}

/// Build the `Sensor = BATTERY` variant of the sensor data frame: state of charge, current,
/// voltage and (derived) power.
pub fn battery_data_frame(soc_percent: f32, current_a: f32, voltage_v: f32) -> Frame {
    // BattCurrent/BattVoltage are full 16-bit fields, so the generated setters' own float->int
    // cast already saturates at exactly their representable range — no separate clamp needed.
    // BattPower is only 14 of those 16 bits, so it's clamped here to its true representable
    // range (-8192..=8191 raw, i.e. *0.0625) to avoid silently wrapping instead of saturating.
    let soc_raw = ((soc_percent / 100.0) * 255.0).clamp(0.0, 255.0) as u8;
    let power_w = (current_a * voltage_v).clamp(-512.0, 511.9375);

    let mut sensor = dbc_gen::LpPcb03DSensorM0::new();
    sensor.set_batt_so_c(soc_raw).ok().unwrap();
    sensor.set_batt_current(current_a).ok().unwrap();
    sensor.set_batt_voltage(voltage_v).ok().unwrap();
    sensor.set_batt_power(power_w).ok().unwrap();

    let mut frame = dbc_gen::LpPcb03D::new(0).ok().unwrap();
    frame.set_m0(sensor).ok().unwrap();
    Frame::new_standard(dbc_gen::LpPcb03D::MESSAGE_ID as u16, frame.raw()).ok().unwrap()
}

/// Build the `Sensor = TEMPERATURE` variant of the sensor data frame.
pub fn temperature_data_frame(temperature_c: f32) -> Frame {
    // TelemetryBattTemperature is a 12-bit field within a 16-bit intermediate cast, so (like
    // BattPower above) it's clamped to its true representable range (-2048..=2047 raw, i.e.
    // *0.0625) rather than relying on the wider intermediate cast's saturation.
    let temperature_c = temperature_c.clamp(-128.0, 127.9375);

    let mut sensor = dbc_gen::LpPcb03DSensorM1::new();
    sensor.set_telemetry_batt_temperature(temperature_c).ok().unwrap();

    let mut frame = dbc_gen::LpPcb03D::new(1).ok().unwrap();
    frame.set_m1(sensor).ok().unwrap();
    Frame::new_standard(dbc_gen::LpPcb03D::MESSAGE_ID as u16, frame.raw()).ok().unwrap()
}

/// Build our own LP_PCB03_P CurrentState confirmation (MessageType=1).
pub fn state_frame(state: CurrentState) -> Frame {
    let current_state = match state {
        CurrentState::Idle => dbc_gen::LpPcb03PCurrentState::Idle,
        CurrentState::Running => dbc_gen::LpPcb03PCurrentState::Running,
    };
    let mut m1 = dbc_gen::LpPcb03PMessageTypeM1::new();
    m1.set_current_state(current_state.into()).ok().unwrap();

    let mut frame = dbc_gen::LpPcb03P::new(1).ok().unwrap();
    frame.set_m1(m1).ok().unwrap();
    Frame::new_standard(dbc_gen::LpPcb03P::MESSAGE_ID as u16, frame.raw()).ok().unwrap()
}

/// Decode an incoming LP_PCB05_P payload's CurrentState. Returns `None` if the frame isn't
/// currently announcing CurrentState (MessageType != 1, e.g. it's a ProcedureStatus or Command).
pub fn decode_dashboard_state(data: &[u8]) -> Option<DashboardState> {
    let mut frame = dbc_gen::LpPcb05P::try_from(data).ok()?;
    let dbc_gen::LpPcb05PMessageType::M1(m1) = frame.message_type().ok()? else {
        return None;
    };
    match m1.current_state() {
        dbc_gen::LpPcb05PCurrentState::Idle => Some(DashboardState::Idle),
        dbc_gen::LpPcb05PCurrentState::Starting => Some(DashboardState::Starting),
        dbc_gen::LpPcb05PCurrentState::Started => Some(DashboardState::Started),
        dbc_gen::LpPcb05PCurrentState::ShuttingDown => Some(DashboardState::ShuttingDown),
        _ => None,
    }
}
