use crate::modem::{Modem, ModemReady, DriverError};
use heapless::String;

pub struct NbModem<U, RST, PWR, VINT> {
    pub modem: Modem<U, RST, PWR, VINT>, 
}

// =============================================================
// CORRECTION ICI : L'Enum prend maintenant 2 types d'erreurs
// =============================================================
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ModemError<RxErr, TxErr> {
    ReadError(RxErr),   // Erreur venant de la lecture (Rx)
    WriteError(TxErr),  // Erreur venant de l'écriture (Tx)
    InitializationFailed,
    ResponseTimeout,
    BufferError, 
}

impl<U, RST, PWR, VINT, RxErr, TxErr> NbModem<U, RST, PWR, VINT>
where
    // On définit que U (l'UART) a une erreur RxErr en lecture et TxErr en écriture
    U: embedded_hal::serial::Read<u8, Error = RxErr> + embedded_hal::serial::Write<u8, Error = TxErr>,
    RST: embedded_hal::digital::v2::OutputPin,
    PWR: embedded_hal::digital::v2::OutputPin,
    VINT: embedded_hal::digital::v2::InputPin,
{
    pub fn new(modem: Modem<U, RST, PWR, VINT>) -> Self {
        Self { modem }
    }

    pub fn begin<F>(&mut self, now_ms: &F) -> Result<(), ModemError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        // On mappe les erreurs du Driver (Rx/Tx/Timeout) vers notre ModemError
        match self.modem.begin(false, now_ms) {
            Ok(true) => Ok(()),
            Ok(false) => Err(ModemError::InitializationFailed),
            
            // Conversion des erreurs DriverError -> ModemError
            Err(DriverError::Rx(e)) => Err(ModemError::ReadError(e)),
            Err(DriverError::Tx(e)) => Err(ModemError::WriteError(e)),
            Err(DriverError::Timeout) => Err(ModemError::ResponseTimeout),
        }
    }

    /// Récupère l'IMEI
    pub fn get_imei<F>(&mut self, now_ms: &F) -> Result<String<20>, ModemError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        // Envoi : Erreur Tx
        self.modem
            .send("AT+CGSN", now_ms)
            .map_err(|e| match e {
                DriverError::Tx(err) => ModemError::WriteError(err),
                DriverError::Rx(err) => ModemError::ReadError(err),
                DriverError::Timeout => ModemError::ResponseTimeout,
            })?;

        // Attente réponse : Erreur Rx ou Timeout
        match self.modem.wait_for_response(1000, now_ms) {
            Ok(ModemReady::Ok) => {
                let content = self.clean_response();
                
                let mut imei = String::<20>::new();
                if imei.push_str(content).is_err() {
                    return Err(ModemError::BufferError);
                }
                Ok(imei)
            }
            Ok(_) => Err(ModemError::ResponseTimeout),
            Err(DriverError::Rx(e)) => Err(ModemError::ReadError(e)),
            Err(DriverError::Tx(e)) => Err(ModemError::WriteError(e)),
            Err(DriverError::Timeout) => Err(ModemError::ResponseTimeout),
        }
    }

    /// Récupère l'ICCID
    pub fn get_iccid<F>(&mut self, now_ms: &F) -> Result<String<32>, ModemError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        self.modem
            .send("AT+CCID", now_ms)
            .map_err(|e| match e {
                DriverError::Tx(err) => ModemError::WriteError(err),
                DriverError::Rx(err) => ModemError::ReadError(err),
                DriverError::Timeout => ModemError::ResponseTimeout,
            })?;

        match self.modem.wait_for_response(1000, now_ms) {
            Ok(ModemReady::Ok) => {
                let content = self.clean_response();

                let clean_iccid = if content.starts_with("+CCID: ") {
                    &content[7..] 
                } else {
                    content
                };

                let mut result = String::<32>::new();
                if result.push_str(clean_iccid).is_err() {
                     return Err(ModemError::BufferError);
                }
                Ok(result)
            }
            Ok(_) => Err(ModemError::ResponseTimeout),
            Err(DriverError::Rx(e)) => Err(ModemError::ReadError(e)),
            Err(DriverError::Tx(e)) => Err(ModemError::WriteError(e)),
            Err(DriverError::Timeout) => Err(ModemError::ResponseTimeout),
        }
    }

    fn clean_response(&self) -> &str {
        let buf_str = self.modem.buf.as_str();
        // On cherche le dernier "OK" pour couper juste avant
        let end = buf_str.rfind("OK").unwrap_or(buf_str.len());
        let raw_content = &buf_str[..end];
        raw_content.trim()
    }
}