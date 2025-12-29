#![no_std]

use core::fmt::{self, Write as FmtWrite};
use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::serial::{Read, Write};
use heapless::String;
use nb;

// Constantes de temps (en ms)
pub const MODEM_MIN_RESPONSE_OR_URC_WAIT_MS: u32 = 20;
pub const BUF_CAP: usize = 512;
pub const POWER_ON_PULSE_MS: u32 = 150;
pub const POWER_OFF_PULSE_MS: u32 = 1500;
pub const BOOT_DELAY_MS: u32 = 200;
pub const SHORT_WAIT_MS: u32 = 100;
pub const RESET_PULSE_MS: u32 = 10_000;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModemState {
    Idle,
    ReceivingResponse,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModemReady {
    NotReady = 0,
    Ok = 1,
    Error = 2,
    NoCarrier = 3,
    CmeError = 4,
}

// NOUVEAU : Un wrapper pour séparer les erreurs de lecture et d'écriture
#[derive(Debug, Copy, Clone)]
pub enum DriverError<RxErr, TxErr> {
    Rx(RxErr),
    Tx(TxErr),
    Timeout, // Ajout utile pour les wait
}

pub struct Modem<U, RST, PWR, VINT> {
    uart: U,
    reset: RST,
    power: PWR,
    vint_pin: Option<VINT>,
    pub buf: String<BUF_CAP>,
    state: ModemState,
    last_response_or_urc_ms: u32,
    ready: ModemReady,
}

// CHANGEMENT ICI : On déclare RxErr et TxErr séparément
impl<U, RST, PWR, VINT, RxErr, TxErr> Modem<U, RST, PWR, VINT>
where
    U: Read<u8, Error = RxErr> + Write<u8, Error = TxErr>,
    RST: OutputPin,
    PWR: OutputPin,
    VINT: InputPin,
{
    pub fn new(uart: U, reset: RST, power: PWR, vint_pin: Option<VINT>) -> Self {
        Self {
            uart,
            reset,
            power,
            vint_pin,
            buf: String::new(),
            state: ModemState::Idle,
            last_response_or_urc_ms: 0,
            ready: ModemReady::NotReady,
        }
    }

    // Le Result retourne maintenant DriverError<RxErr, TxErr>
    pub fn begin<F>(&mut self, restart: bool, now_ms: &F) -> Result<bool, DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        let _ = self.power.set_low();
        let _ = self.reset.set_low();

        if restart {
            self.shutdown(now_ms)?;
        }

        let is_powered = if let Some(vint) = &self.vint_pin {
            vint.is_high().unwrap_or(false)
        } else {
            false
        };

        if !is_powered {
            let _ = self.power.set_high();
            self.blocking_wait(POWER_ON_PULSE_MS, now_ms);
            let _ = self.power.set_low();
            self.blocking_wait(SHORT_WAIT_MS, now_ms);
        }

        if self.autosense(now_ms)? {
            return Ok(true);
        }

        Ok(false)
    }

    pub fn shutdown<F>(&mut self, now_ms: &F) -> Result<bool, DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        if self.autosense_timeout(200, now_ms)? {
            self.send("AT+CPWROFF", now_ms)?;
            if let Ok(ModemReady::Ok) = self.wait_for_response(40_000, now_ms) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn end<F>(&mut self, now_ms: &F)
    where
        F: Fn() -> u32,
    {
        let _ = self.power.set_high();
        self.blocking_wait(POWER_OFF_PULSE_MS, now_ms);
        let _ = self.power.set_low();
    }

    pub fn hard_reset<F>(&mut self, now_ms: &F)
    where
        F: Fn() -> u32,
    {
        let _ = self.reset.set_high();
        self.blocking_wait(RESET_PULSE_MS, now_ms);
        let _ = self.reset.set_low();
    }

    pub fn send<F>(&mut self, command: &str, now_ms: &F) -> Result<(), DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        self.prepare_send(now_ms);
        self.write_raw(command.as_bytes())?;
        self.write_raw(b"\r\n")?;
        Ok(())
    }

    pub fn send_fmt<F>(
        &mut self,
        args: fmt::Arguments,
        now_ms: &F,
    ) -> Result<(), DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        self.prepare_send(now_ms);
        let mut temp_buf = String::<128>::new();
        if temp_buf.write_fmt(args).is_ok() {
            self.write_raw(temp_buf.as_bytes())?;
            self.write_raw(b"\r\n")?;
        }
        Ok(())
    }

    fn prepare_send<F>(&mut self, now_ms: &F)
    where
        F: Fn() -> u32,
    {
        let delta = now_ms().wrapping_sub(self.last_response_or_urc_ms);
        if delta < MODEM_MIN_RESPONSE_OR_URC_WAIT_MS {
            self.blocking_wait(MODEM_MIN_RESPONSE_OR_URC_WAIT_MS - delta, now_ms);
        }
        self.buf.clear();
        self.state = ModemState::Idle;
        self.ready = ModemReady::NotReady;
    }

    // Cette fonction écrit (Tx) ET lit (Rx) pour l'écho, donc elle utilise DriverError
    fn write_raw(&mut self, data: &[u8]) -> Result<(), DriverError<RxErr, TxErr>> {
        let mut written = 0;
        for &b in data {
            // Mappe l'erreur d'écriture vers DriverError::Tx
            nb::block!(self.uart.write(b)).map_err(DriverError::Tx)?;
            written += 1;
        }

        let mut ignored = 0;
        while ignored < written {
            match self.uart.read() {
                Ok(_) => ignored += 1,
                Err(nb::Error::WouldBlock) => break,
                // Mappe l'erreur de lecture vers DriverError::Rx
                Err(nb::Error::Other(e)) => return Err(DriverError::Rx(e)),
            }
        }
        Ok(())
    }

    pub fn autosense<F>(&mut self, now_ms: &F) -> Result<bool, DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        self.autosense_timeout(10000, now_ms)
    }

    pub fn autosense_timeout<F>(
        &mut self,
        timeout_ms: u32,
        now_ms: &F,
    ) -> Result<bool, DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        let start = now_ms();
        while now_ms().wrapping_sub(start) < timeout_ms {
            self.send("AT", now_ms)?;
            if let Ok(ModemReady::Ok) = self.wait_for_response(200, now_ms) {
                return Ok(true);
            }
            self.blocking_wait(100, now_ms);
        }
        Ok(false)
    }

    pub fn wait_for_response<F>(
        &mut self,
        timeout_ms: u32,
        now_ms: &F,
    ) -> Result<ModemReady, DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        let start = now_ms();
        while now_ms().wrapping_sub(start) < timeout_ms {
            self.poll(now_ms)?;
            if self.ready != ModemReady::NotReady {
                return Ok(self.ready);
            }
        }
        Ok(ModemReady::NotReady)
    }

    // poll ne fait que lire, mais pour uniformiser l'API, on garde DriverError
    pub fn poll<F>(&mut self, now_ms: &F) -> Result<(), DriverError<RxErr, TxErr>>
    where
        F: Fn() -> u32,
    {
        loop {
            match self.uart.read() {
                Ok(byte) => {
                    let c = byte as char;
                    if self.buf.len() < self.buf.capacity() {
                        let _ = self.buf.push(c);
                    }

                    match self.state {
                        ModemState::Idle => {
                            if self.buf.ends_with("\r\n") {
                                let trimmed = self.buf.trim();
                                if trimmed.len() > 0 {
                                    self.last_response_or_urc_ms = now_ms();

                                    if trimmed == "AT" {
                                        self.state = ModemState::ReceivingResponse;
                                        self.buf.clear();
                                    } else {
                                        // URC traité (ignoré pour l'instant)
                                        self.buf.clear();
                                    }
                                } else {
                                    self.buf.clear();
                                }

                                if self.buf.starts_with("AT") && self.buf.contains("\r\n") {
                                    self.state = ModemState::ReceivingResponse;
                                }
                            }
                        }
                        ModemState::ReceivingResponse => {
                            if c == '\n' {
                                self.last_response_or_urc_ms = now_ms();

                                let is_ok = self.buf.ends_with("OK\r\n");
                                let is_err = self.buf.ends_with("ERROR\r\n");
                                let is_no_carrier = self.buf.ends_with("NO CARRIER\r\n");
                                let is_cme = self.buf.contains("CME ERROR");

                                if is_ok {
                                    self.ready = ModemReady::Ok;
                                } else if is_err {
                                    self.ready = ModemReady::Error;
                                } else if is_no_carrier {
                                    self.ready = ModemReady::NoCarrier;
                                } else if is_cme {
                                    self.ready = ModemReady::CmeError;
                                }

                                if self.ready != ModemReady::NotReady {
                                    self.state = ModemState::Idle;
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(DriverError::Rx(e)),
            }
        }
        Ok(())
    }

    fn blocking_wait<F>(&self, ms: u32, now_ms: &F)
    where
        F: Fn() -> u32,
    {
        let start = now_ms();
        while now_ms().wrapping_sub(start) < ms {}
    }
}
