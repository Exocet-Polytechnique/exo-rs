use defmt::error;
use embassy_stm32::can::{Can, Frame};
use embassy_time::Timer;

use crate::{tasks::can::{CanData, DATA_CHANNEL, ERROR_CHANNEL, ErrorType, WARNING_CHANNEL, WarningType}};

const TICK_DELAY_MS: u64 = 1000;
const START_CMD: [u8; 8] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
const RESISTANCE_THRESHOLD: u32 = 1_000_000;

// async fn send_command(target: dbc_gen::LpPcb03PTargetModule, command: dbc_gen::LpPcb03PCommand) {
//     let mut m2 = dbc_gen::LpPcb01PMessageTypeM2::new();

//     if m2.set_target_module(target.into()).is_err() {
//         return;
//     }

//     if m2.set_command(command.into()).is_err() {
//         return;
//     }

//     if let Ok(mut lp_p) = dbc_gen::LpPcb01P::new(0) {
//         if lp_p.set_m2(m2).is_ok() {
//             if let Ok(f) = Frame::new_standard(dbc_gen::LpPcb01P::MESSAGE_ID as u16, lp_p.raw()) {
//                 //tx.write(&f).await;     
//                 DATA_CHANNEL.send( CanData {})
//             }
//         }
//     }

// }


#[embassy_executor::task]
pub async fn imd_task(mut can_bus: Can<'static>) {
    let frame = Frame::new_extended(0x1819A1A5, &START_CMD).unwrap();
    can_bus.write(&frame).await;

    loop {
        match can_bus.read().await {
            Ok(envelope) => {
                let _id = match envelope.frame.id() {
                    embedded_can::Id::Standard(id) => id.as_raw() as u32,
                    embedded_can::Id::Extended(id) => id.as_raw(),
                };

                let data = envelope.frame.data();

                if data.len() != 8 {
                    continue;
                }

                let r_iso_plus: u16 = ((data[1] as u16) << 8) | (data[2] as u16);
                let r_iso_minus: u16 = ((data[5] as u16) << 8) | (data[6] as u16);

                let r_iso_min = r_iso_plus.min(r_iso_minus);

                let resistance_ohms: u32 = (r_iso_min as u32) * 8192;

                let is_fault = resistance_ohms < RESISTANCE_THRESHOLD;

                if (is_fault) {
                    ERROR_CHANNEL.send(ErrorType::IsolationFault).await;
                }

                DATA_CHANNEL.send(CanData::ImdStatus(is_fault)).await;
                DATA_CHANNEL.send(CanData::ImdResistance((r_iso_min) as f32)).await;
            }
            Err(_) => error!("CAN read error"),
        }

        Timer::after_millis(TICK_DELAY_MS).await;
    }
}
