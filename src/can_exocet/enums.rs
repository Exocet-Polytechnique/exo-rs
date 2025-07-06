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
    StateControl = 0x01, 
    HydrogenManagement = 0x02,
    FcControllers = 0x03,
    Power = 0x04,
    Batteries = 0x05,
    Telemetry = 0x06,
}



