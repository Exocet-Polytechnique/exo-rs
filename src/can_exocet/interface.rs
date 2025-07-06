use core::fmt::Error;

use super::enums::{Subsystem, Priority};
use super::errors::CanError;
use embassy_stm32::can::{self, Can, Timestamp, Frame};

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
        subsystem: Subsystem,
        priority: Priority,
        module: i8,
        local_priority: bool,
        payload: &[u8],
    ) -> Result<(), CanError> {

        if module > 0b11 {
            return Err(CanError::InvalidModule);
        }

        let priority_bits = (priority as u16 & 0b11) << 9;
        let subsystem_bits = (subsystem as u16 & 0b11111) << 4;
        let local_bit = (local_priority as u16 & 0b1) << 3;
        let module_bits = module & 0b111;

        let id= priority_bits | subsystem_bits | local_bit | module_bits as u16;

        let frame = can::frame::Frame::new_standard(id, payload).unwrap();
        _ = self.can.write(&frame).await.ok_or(CanError::FrameError)?;

        if let Some(_dropped_frame) = self.can.write(&frame).await {
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
