use embassy_stm32::can;
use embassy_stm32::can::Frame;
use embassy_stm32::can::enums::FrameCreateError;

#[derive(Debug, Copy, Clone)]
pub enum Priority {
    InfoMessage = 0b11,
    StateManagementMessage = 0b10,
    WarningMessage = 0b01,
    CriticalErrorMessage = 0b00,
}

#[derive(Debug, Copy, Clone)]
pub enum Subsystem {
    Reserved = 0x00,
    Broadcast = 0x1F,
    ControlCockpit = 0x08, // good 
    HydrogenManagement = 0x10, // in binary 0001 0000
    FcControllersA = 0x18, // in binary 0001 1000
    FcControllersB = 0x19, // in binary 0001 1001
    PowerHighPower = 0x20, // in binary 0010 0000
    PowerDcDc = 0x21, // in binary 0010 0001
    Batteries1 = 0x28,  // in binary 0010 1000
    TelemetryScreen = 0x30, // in binary 0011 0000
    TelemetryTelemetry = 0x31, // in binary 0011 0001
}



#[derive(Debug, Copy, Clone)]
pub struct CanModule {
    pub priority: Priority,
    pub subsystem: Subsystem,
}

impl CanModule {
    pub fn new(priority: Priority, subsystem: Subsystem) -> Self {
        Self { priority, subsystem }
    }

    pub fn CanToId(&self) -> u16 {
        ((self.priority as u16) << 9) |
        ((self.subsystem_module as u16) << 2)
    }

    pub fn IdToCan(id: u16) -> Self {
        let priority = match (id >> 9) & 0b11 {
            0b11 => Priority::InfoMessage,
            0b10 => Priority::StateManagementMessage,
            0b01 => Priority::WarningMessage,
            0b00 => Priority::CriticalErrorMessage,
            _ => unreachable!(),
        };
        let subsystem_module = match (id >> 2) & 0xFF {
            0x00 => SubsystemModule::Reserved,
            0x1F => SubsystemModule::Broadcast,
            0x08 => SubsystemModule::ControlCockpit,
            0x10 => SubsystemModule::HydrogenManagement,
            0x18 => SubsystemModule::FcControllersA,
            0x19 => SubsystemModule::FcControllersB,
            0x20 => SubsystemModule::PowerHighPower,
            0x21 => SubsystemModule::PowerDcDc,
            0x28 => SubsystemModule::Batteries1,
            0x30 => SubsystemModule::TelemetryScreen,
            0x31 => SubsystemModule::TelemetryTelemetry,
            _ => unreachable!(),
        };
        Self { priority, subsystem_module }
    }
    Into<Frame> for CanModule {
        fn into(self, data) -> Frame {
            Frame::new_standard(self.CanToId(), data).unwrap()
        }
    }
    impl From<Frame> for CanModule {
        fn from(frame: Frame) -> Self {
            Self::IdToCan(frame.id())
        }
    }
}

pub async fn create_frame(module: CanModule, data: [u8; 8]) -> Result<Frame, FrameCreateError> {
    let id = module.CanToId();
    let frame = can::Frame::new_standard(id, &data);
    Ok(frame?)
}