use core::u8;

use crate::can_exocet;
use crate::can_exocet::enums::{DataSubtype, FrameType, PingSubtype, ProcedureSubtype, StateSubtype};

use super::enums::{Subsystem, Priority};
use super::errors::CanError;
use embassy_stm32::can::{self, Can, Timestamp, Frame};

const VALID_ADDRESSES: [u8; 8] = [0x00, 0xFF, 0x08, 0x10, 0x18, 0x20, 0x28, 0x29];

pub struct CanDriver<'a>{
    can: Can<'a>,
    last_read_ts: Option<Timestamp>,
    last_frame: Option<Frame>,
}

impl<'a> CanDriver<'a> {

    pub fn new(can: Can<'a>) -> Self {
        Self { can, last_read_ts: None, last_frame: None }
    }

    pub async fn send_message(
        &mut self,
        frame: CanFrameExocet,
    ) -> Result<(), CanError> {

        if frame.module > 0b11 {
            return Err(CanError::InvalidModule);
        }

        let priority_bits = (frame.priority as u16 & 0b11) << 9;
        let subsystem_bits = (frame.subsystem as u16 & 0b11111) << 4;
        let local_bit = (frame.local_priority as u16 & 0b1) << 3;
        let module_bits = frame.module & 0b111;

        let id= priority_bits | subsystem_bits | local_bit | module_bits as u16;

        let can_frame = can::frame::Frame::new_standard(id, &frame.payload).unwrap();
        _ = self.can.write(&can_frame).await;

        if let Some(_dropped_frame) = self.can.write(&can_frame).await {
        return Err(CanError::DroppedFrame);
        }

        Ok(())
    }

    pub async fn read_message(&mut self) -> Result<(CanFrameExocet, Timestamp), CanError> {
        match self.can.read().await {
            Ok(envelope) => {
                let (frame, ts) = envelope.parts();

                self.last_read_ts = Some(ts);
                self.last_frame = Some(frame);

                 let id_raw: u16 = match frame.id() {
                    embedded_can::Id::Standard(id) => id.as_raw(),
                    embedded_can::Id::Extended(_) => {
                        return Err(CanError::ExtendedHeaderReceived);
                    }
                };
                let priority = (id_raw >> 9) & 0b11;
                let address = (id_raw >> 4) & 0b1_1111;
                let local_priority = (id_raw >> 3) & 0b1;
                let submodule = id_raw & 0b111;

                let data_slice: &[u8] = frame.data();
                let mut payload: [u8; 8] = [0; 8];
                payload[..data_slice.len()].copy_from_slice(data_slice);

                let _can_frame = CanFrameExocet {
                    payload: payload,
                    priority: match priority {
                        0b00 => Priority::CriticalErrorMessage,
                        0b01 => Priority::WarningMessage,
                        0b10 => Priority::StateManagementMessage,
                        0b11 => Priority::InfoMessage,
                        _ => unreachable!(),
                    },
                    subsystem: match address {
                        0x00 => Subsystem::Reserved,
                        0x1F => Subsystem::Broadcast,
                        0x01 => Subsystem::StateControl,
                        0x02 => Subsystem::HydrogenManagement,
                        0x03 => Subsystem::HighPower,
                        0x04 => Subsystem::LowPower,
                        0x05 => Subsystem::Telemetry,
                        _ => return Err(CanError::InvalidModule),
                    },
                    local_priority: (local_priority != 0),
                    module: submodule as u8,
                };

                Ok((_can_frame, ts))

            },
            Err(_) => Err(CanError::FrameError),
        }
    }

    pub fn last_read_timestamp(&self) -> Option<Timestamp> {
        self.last_read_ts
    }

    pub fn last_frame(&self) -> Option<Frame> {
        self.last_frame
    }
    
}

pub struct CanFrameExocet {
    payload: [u8; 8],
    priority: Priority,
    subsystem: Subsystem,
    local_priority: bool,
    module: u8,
}

impl CanFrameExocet {
    pub fn new(frame_type: FrameType, data: u64, priority: Priority, subsystem: Subsystem, local_priority: bool, module: u8) -> Result<Self, CanError> {
        match frame_type {
            FrameType::State(subtype) => {
                match subtype {
                    StateSubtype::StateAnnouncement => {
                        //Takes State as data
                        if data > 4 { // Assuming there are about 5 states (TO BE DEFINED)
                           return Err(CanError::InvalidPayload);
                       }

                       let mut payload = [0; 8];
                        payload[0] = data as u8;

                        Ok(Self {payload, priority, subsystem, local_priority, module})
                    },
                    StateSubtype::RequestConfirmation => {
                        // Takes Address of targeted module as data
                        if !VALID_ADDRESSES.contains(&(data as u8)) {
                            return Err(CanError::InvalidPayload);
                        }
                        let mut payload = [0; 8];
                        payload[0] = data as u8;

                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                    StateSubtype::StateRequest => {
                        // No data: Request to Cockpit
                        let payload = [0; 8];

                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                    StateSubtype::StateConfirmation => {
                        // No data: Acknowledgement from module
                        let payload = [0; 8];

                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                }
            },
            FrameType::Ping(ping_subtype) => {
                match ping_subtype {
                    PingSubtype::PresenceRequest => {
                        // Takes Address of targeted module as data
                        if !VALID_ADDRESSES.contains(&(data as u8)) {
                            return Err(CanError::InvalidPayload);
                        }
                        let mut payload = [0; 8];
                        payload[0] = data as u8;

                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                    PingSubtype::PresenceAnnouncement => {
                        // No data: Acknowledgement from module
                        let payload = [0; 8];

                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                }
            },
            FrameType::Data(data_subtype) => {
                match data_subtype {
                    DataSubtype::DataRequest => {
                        // Takes Address of targeted module as data
                        if !VALID_ADDRESSES.contains(&(data as u8)) {
                            return Err(CanError::InvalidPayload);
                        }
                        let mut payload = [0; 8];
                        payload[0] = data as u8;
                        // Verify for internal addresses (addresses for sensors within a module TO BE DEFINED)
                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                    DataSubtype::DataAnnouncement => {
                        // No data: DataAnnouncement payload must be 0 bytes
                        let payload = [0; 8];
                        Ok(Self { payload, priority, subsystem, local_priority, module })
                    },
                }
            },
            FrameType::Procedure(procedure_subtype) => {
                match procedure_subtype {
                    ProcedureSubtype::ProcedureControl => {
                        // Takes procedure id and procedure action
                        // Assign 7 bytes to id and last byte to action
                        let last_byte: u8 = (data & 0xFF) as u8;
                        if last_byte > 3 {
                            return Err(CanError::InvalidPayload);
                        }
                        let payload = data.to_be_bytes();
                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                    ProcedureSubtype::ProcedureRequest => {
                        // Takes prodcedure id as data (usually broadcasted so no address needed for now)
                        let payload = data.to_be_bytes();
                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                    ProcedureSubtype::ProcedureResponse => {
                        // Takes procedure state as data
                        if data > 3 {
                            return Err(CanError::InvalidPayload);
                        }
                        let payload = data.to_be_bytes();
                        Ok(Self{payload, priority, subsystem, local_priority, module})
                    },
                }
            },
        }
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    pub fn subsystem(&self) -> Subsystem {
        self.subsystem
    }   

    pub fn local_priority(&self) -> bool {
        self.local_priority
    }

    pub fn payload(&self) -> [u8; 8] {
        self.payload
    }

    pub fn module(&self) -> u8 {
        self.module
    }
}
