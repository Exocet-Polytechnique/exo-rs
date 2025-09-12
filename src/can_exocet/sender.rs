use core::u8;

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
        priority: Priority,
        subsystem: Subsystem,
        module: i8,
        local_priority: bool,
        frame: CanFrameExocet,
    ) -> Result<(), CanError> {

        if module > 0b11 {
            return Err(CanError::InvalidModule);
        }

        let priority_bits = (priority as u16 & 0b11) << 9;
        let subsystem_bits = (subsystem as u16 & 0b11111) << 4;
        let local_bit = (local_priority as u16 & 0b1) << 3;
        let module_bits = module & 0b111;

        let id= priority_bits | subsystem_bits | local_bit | module_bits as u16;

        let can_frame = can::frame::Frame::new_standard(id, &frame.payload).unwrap();
        _ = self.can.write(&can_frame).await;

        if let Some(_dropped_frame) = self.can.write(&can_frame).await {
        return Err(CanError::DroppedFrame);
        }

        Ok(())
    }

    pub async fn read_message(&mut self) -> Result<(Frame, Timestamp), CanError> {
        match self.can.read().await {
            Ok(envelope) => {
                let (frame, ts) = envelope.parts();

                self.last_read_ts = Some(ts);
                self.last_frame = Some(frame);

                Ok((frame, ts))
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
}

impl CanFrameExocet {
    pub fn new(frame_type: FrameType, data: u64 ) -> Result<Self, CanError> {
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

                        Ok(Self {payload})
                    },
                    StateSubtype::RequestConfirmation => {
                        // Takes Address of targeted module as data
                        if !VALID_ADDRESSES.contains(&(data as u8)) {
                            return Err(CanError::InvalidPayload);
                        }
                        let mut payload = [0; 8];
                        payload[0] = data as u8;

                        Ok(Self{payload})
                    },
                    StateSubtype::StateRequest => {
                        // No data: Request to Cockpit
                        let payload = [0; 8];

                        Ok(Self{payload})
                    },
                    StateSubtype::StateConfirmation => {
                        // No data: Acknowledgement from module
                        let payload = [0; 8];

                        Ok(Self{payload})
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

                        Ok(Self{payload})
                    },
                    PingSubtype::PresenceAnnouncement => {
                        // No data: Acknowledgement from module
                        let payload = [0; 8];

                        Ok(Self{payload})
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
                        Ok(Self{payload})
                    },
                    DataSubtype::DataAnnouncement => {
                        // No data: DataAnnouncement payload must be 0 bytes
                        let payload = [0; 8];
                        Ok(Self { payload })
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
                        Ok(Self{payload})
                    },
                    ProcedureSubtype::ProcedureRequest => {
                        // Takes prodcedure id as data (usually broadcasted so no address needed for now)
                        let payload = data.to_be_bytes();
                        Ok(Self{payload})
                    },
                    ProcedureSubtype::ProcedureResponse => {
                        // Takes procedure state as data
                        if data > 3 {
                            return Err(CanError::InvalidPayload);
                        }
                        let payload = data.to_be_bytes();
                        Ok(Self{payload})
                    },
                }
            },
        }
    }
}
