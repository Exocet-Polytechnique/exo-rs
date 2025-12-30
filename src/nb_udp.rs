#![no_std]

use crate::modem::{DriverError, Modem, ModemReady};
use crate::nb_modem::ModemError; // Importation cruciale pour résoudre l'erreur
use core::fmt::Write;
use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::serial::{Read, Write as SerialWrite};
use heapless::String;

pub const UDP_TX_BUF_SIZE: usize = 512;
pub const UDP_RX_BUF_SIZE: usize = 512;

pub struct NbUdp<'a, U, RST, PWR, VINT> {
    modem: &'a mut Modem<U, RST, PWR, VINT>,
    socket: i8,
    packet_received: bool,
    tx_ip: Option<[u8; 4]>,
    tx_port: u16,
    tx_buffer: [u8; UDP_TX_BUF_SIZE],
    tx_size: usize,
    rx_ip: [u8; 4],
    rx_port: u16,
    rx_buffer: [u8; UDP_RX_BUF_SIZE],
    rx_size: usize,
    rx_index: usize,
}

impl<'a, U, RST, PWR, VINT, RxErr, TxErr> NbUdp<'a, U, RST, PWR, VINT>
where
    U: Read<u8, Error = RxErr> + SerialWrite<u8, Error = TxErr>,
    RST: OutputPin,
    PWR: OutputPin,
    VINT: InputPin,
{
    pub fn new(modem: &'a mut Modem<U, RST, PWR, VINT>) -> Self {
        Self {
            modem,
            socket: -1,
            packet_received: false,
            tx_ip: None,
            tx_port: 0,
            tx_buffer: [0; UDP_TX_BUF_SIZE],
            tx_size: 0,
            rx_ip: [0; 4],
            rx_port: 0,
            rx_buffer: [0; UDP_RX_BUF_SIZE],
            rx_size: 0,
            rx_index: 0,
        }
    }

    /// Ouvre un socket UDP (Équivalent de NBUDP::begin)
    pub fn begin<F>(&mut self, port: u16, now_ms: &F) -> Result<bool, ModemError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        // AT+USOCR=17 (17 = UDP)
        self.modem
            .send("AT+USOCR=17", now_ms)
            .map_err(Self::map_err)?;

        if let ModemReady::Ok = self
            .modem
            .wait_for_response(2000, now_ms)
            .map_err(Self::map_err)?
        {
            // Extraction du numéro de socket depuis le buffer (ex: "+USOCR: 0")
            let resp = self.modem.buf.as_str();
            self.socket = resp
                .chars()
                .filter(|c| c.is_digit(10))
                .next()
                .and_then(|c| c.to_digit(10))
                .unwrap_or(0) as i8;

            // Liaison au port local : AT+USOLI=<socket>,<port>
            let mut cmd = String::<32>::new();
            write!(cmd, "AT+USOLI={},{}", self.socket, port).ok();
            self.modem.send(&cmd, now_ms).map_err(Self::map_err)?;

            if let ModemReady::Ok = self
                .modem
                .wait_for_response(10000, now_ms)
                .map_err(Self::map_err)?
            {
                return Ok(true);
            }
        }

        self.stop(now_ms)?;
        Ok(false)
    }

    pub fn stop<F>(&mut self, now_ms: &F) -> Result<(), ModemError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        if self.socket >= 0 {
            let mut cmd = String::<32>::new();
            write!(cmd, "AT+USOCL={},1", self.socket).ok(); //
            self.modem.send(&cmd, now_ms).map_err(Self::map_err)?;
            self.modem
                .wait_for_response(10000, now_ms)
                .map_err(Self::map_err)?;
            self.socket = -1;
        }
        Ok(())
    }

    /// Prépare l'envoi d'un paquet (Équivalent de NBUDP::beginPacket)
    pub fn begin_packet(&mut self, ip: [u8; 4], port: u16) -> bool {
        if self.socket < 0 {
            return false;
        }
        self.tx_ip = Some(ip);
        self.tx_port = port;
        self.tx_size = 0;
        true
    }

    /// Envoie les données accumulées (Équivalent de NBUDP::endPacket)
    pub fn end_packet<F>(&mut self, now_ms: &F) -> Result<bool, ModemError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        if let Some(ip) = self.tx_ip {
            let mut cmd = String::<1024>::new();
            // AT+USOST=<socket>,"<ip>",<port>,<length>,"<hex_data>"
            write!(
                cmd,
                "AT+USOST={},\"{}.{}.{}.{}\",{},{},\"",
                self.socket, ip[0], ip[1], ip[2], ip[3], self.tx_port, self.tx_size
            )
            .ok();

            // Conversion Hexadécimale
            for i in 0..self.tx_size {
                write!(cmd, "{:02X}", self.tx_buffer[i]).ok();
            }
            cmd.push('"').ok();

            self.modem.send(&cmd, now_ms).map_err(Self::map_err)?;
            if let ModemReady::Ok = self
                .modem
                .wait_for_response(5000, now_ms)
                .map_err(Self::map_err)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn write(&mut self, data: &[u8]) -> usize {
        let available = UDP_TX_BUF_SIZE - self.tx_size;
        let to_write = core::cmp::min(data.len(), available);
        self.tx_buffer[self.tx_size..self.tx_size + to_write].copy_from_slice(&data[..to_write]);
        self.tx_size += to_write;
        to_write
    }

    fn map_err(e: DriverError<RxErr, TxErr>) -> ModemError<RxErr, TxErr> {
        match e {
            DriverError::Rx(err) => ModemError::ReadError(err),
            DriverError::Tx(err) => ModemError::WriteError(err),
            DriverError::Timeout => ModemError::ResponseTimeout,
        }
    }
}
