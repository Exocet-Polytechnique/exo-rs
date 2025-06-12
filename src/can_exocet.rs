#[derive(Debug, Copy, Clone)]
pub enum Priority {
    InfoMessage = 0b11,
    StateManagementMessage = 0b10,
    WarningMessage = 0b01,
    CriticalErrorMessage = 0b00,
}

impl Priority {
    pub fn to_u32(self) -> u32 {
        self as u32
    }
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

impl Subsystem {
    pub fn to_u32(self) -> u32 {
        self as u32
    }
}