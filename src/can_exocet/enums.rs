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

#[derive(Debug, Copy, Clone)]
pub enum StateSubtype {
    StateAnnouncement = 0x00,
    RequestConfirmation = 0x01,
    StateRequest = 0x02,
    StateConfirmation = 0x03,
}

#[derive(Debug, Copy, Clone)]
pub enum PingSubtype {
    PresenceRequest = 0x00,
    PresenceAnnouncement = 0x02,
}

#[derive(Debug, Copy, Clone)]
pub enum DataSubtype {
    DataRequest = 0x00,
    DataAnnouncement = 0x02,
}

#[derive(Debug, Copy, Clone)]
pub enum ProcedureSubtype {
    ProcedureControl = 0x00,
    ProcedureRequest = 0x01,
    ProcedureResponse = 0x02,
}

pub enum FrameType{
    State(StateSubtype),
    Ping(PingSubtype),
    Data(DataSubtype),
    Procedure(ProcedureSubtype)
}

impl FrameType {
    pub fn type_code(&self) -> u8 {
        match self {
            FrameType::State(_) => FrameType::State as u8,
            FrameType::Ping(_) => FrameType::Ping as u8,
            FrameType::Data(_) => FrameType::Data as u8,
            FrameType::Procedure(_) => FrameType::Procedure as u8,
        }
    }

    pub fn subtype_code(&self) -> u8 {
        match self {
            FrameType::Ping(sub) => *sub as u8,
            FrameType::State(sub) => *sub as u8,
            FrameType::Data(sub) => *sub as u8,
            FrameType::Procedure(sub) => *sub as u8,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ProcedureState {
    Ready = 0x00,
    Running = 0x01,
    Suspended = 0x02,
    Terminated = 0x03
}

#[derive(Debug, Copy, Clone)]
pub enum ProcedureAction {
    Start = 0x00,
    Suspend = 0x01,
    Resume = 0x02,
    Terminate = 0x03
}







