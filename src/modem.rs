#![no_std]

use core::fmt::Write as FmtWrite;

use embedded_hal::serial::{Read, Write};
use embedded_hal::digital::v2::{InputPin, OutputPin};

use heapless::String;
use nb;
use nb::Error as NbError;

pub const MODEM_MIN_RESPONSE_OR_URC_WAIT_MS: u32 = 20;
pub const BUF_CAP: usize = 512;
pub const POWER_ON_PULSE_MS: u32 = 200;
pub const BOOT_DELAY_MS: u32 = 200;
pub const SHORT_WAIT_MS: u32 = 50;
pub const RESET_PULSE_MS: u32 = 50;
pub const HARD_RESET_LOW_MS: u32 = 10_000;

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

pub struct Modem<U, RST, PWR, VINT> {
    uart: U,
    reset: RST,
    power: PWR,
    vint_pin: Option<VINT>, // input-only: used to detect power state if available
    buf: String<BUF_CAP>,
    state: ModemState,
    last_response_or_urc_ms: u32,
    ready: ModemReady,
    // TODO: URC handlers (Vec/array of fn(&str) or trait objects) — left out for now
}

impl<U, RST, PWR, VINT, E> Modem<U, RST, PWR, VINT>
where
    U: Read<u8, Error = E> + Write<u8, Error = E>,
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

    /// begin expects the caller to provide a monotonic function `now_ms` (e.g. systick)
    /// passed as a reference. This design avoids moving closures around.
    pub fn begin<F>(&mut self, restart: bool, now_ms: &F) -> Result<bool, E>
    where
        F: Fn() -> u32,
    {
        if restart {
            // toggle reset quickly (like the original attempted)
            let _ = self.reset.set_low();
            let t0 = now_ms();
            while now_ms().wrapping_sub(t0) < RESET_PULSE_MS {}
            let _ = self.reset.set_high();
            let t1 = now_ms();
            while now_ms().wrapping_sub(t1) < RESET_PULSE_MS {}
        }

        // detect power via vint_pin if present
        let powered = match &self.vint_pin {
            Some(v) => match v.is_high() {
                Ok(high) => high,
                Err(_) => false,
            },
            None => false,
        };

        if !powered {
            // pulse power pin (active HIGH) per datasheet >=150ms
            let _ = self.power.set_high();
            let t0 = now_ms();
            while now_ms().wrapping_sub(t0) < POWER_ON_PULSE_MS {}
            let _ = self.power.set_low();
            // small settle
            let t1 = now_ms();
            while now_ms().wrapping_sub(t1) < SHORT_WAIT_MS {}
        } else {
            // debounce / settle if already powered
            let t = now_ms();
            while now_ms().wrapping_sub(t) < MODEM_MIN_RESPONSE_OR_URC_WAIT_MS {}
        }

        // wait modem boot
        let tboot = now_ms();
        while now_ms().wrapping_sub(tboot) < BOOT_DELAY_MS {}

        // basic autosense: send AT + wait OK
        self.send("AT", now_ms)?;

        match self.wait_for_response_blocking(1000, now_ms)? {
            ModemReady::Ok => Ok(true),
            _ => {
                // retry once quickly
                let t_retry = now_ms();
                while now_ms().wrapping_sub(t_retry) < SHORT_WAIT_MS {}
                self.send("AT", now_ms)?;
                if let ModemReady::Ok = self.wait_for_response_blocking(1000, now_ms)? {
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Shutdown issues an AT+CPWROFF and waits. Note: we cannot toggle a VINT
    /// if it's typed as InputPin. If you need to actively drive VINT,
    /// make that a separate OutputPin generic.
    pub fn shutdown<F>(&mut self, now_ms: &F) -> Result<bool, E>
    where
        F: Fn() -> u32,
    {
        self.send("AT+CPWROFF", now_ms)?;

        match self.wait_for_response_blocking(40_000, now_ms)? {
            ModemReady::Ok => {
                // We can't set vint_pin low because it is InputPin in this signature.
                // If you need to actively drive VINT, change VINT bound to OutputPin or add separate control pin.
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// send an AT-like line (appends CRLF). now_ms passed by reference.
    pub fn send<F>(&mut self, s: &str, now_ms: &F) -> Result<(), E>
    where
        F: Fn() -> u32,
    {
        // enforce minimum inter-command gap by busy-wait (caller could also sleep)
        let delta = now_ms().wrapping_sub(self.last_response_or_urc_ms);
        if delta < MODEM_MIN_RESPONSE_OR_URC_WAIT_MS {
            let wait = MODEM_MIN_RESPONSE_OR_URC_WAIT_MS - delta;
            let t0 = now_ms();
            while now_ms().wrapping_sub(t0) < wait {}
        }

        for &b in s.as_bytes() {
            nb::block!(self.uart.write(b))?;
        }
        nb::block!(self.uart.write(b'\r'))?;
        nb::block!(self.uart.write(b'\n'))?;

        // Reset state machine to expect response
        self.state = ModemState::Idle;
        self.ready = ModemReady::NotReady;
        Ok(())
    }

    /// write binary and discard echoed bytes up to `data.len()`
    pub fn write_and_discard_echo<F>(&mut self, data: &[u8], _now_ms: &F) -> Result<(), E>
    where
        F: Fn() -> u32,
    {
        let mut written = 0usize;
        for &b in data {
            nb::block!(self.uart.write(b))?;
            written += 1;
        }

        // read and discard up to `written` echoed bytes
        let mut ignored = 0usize;
        loop {
            match self.uart.read() {
                Ok(_) => {
                    ignored += 1;
                    if ignored >= written {
                        break;
                    }
                }
                Err(NbError::WouldBlock) => break,
                Err(NbError::Other(e)) => return Err(e),
            }
        }
        Ok(())
    }

    /// poll must be called frequently from your main loop/RTIC task.
    /// It consumes available bytes and updates `ready` and `buf`.
    pub fn poll<F>(&mut self, now_ms: &F) -> Result<(), E>
    where
        F: Fn() -> u32,
    {
        loop {
            match self.uart.read() {
                Ok(b) => {
                    // AT responses are ASCII — casting byte->char is okay here.
                    if self.buf.push(b as char).is_err() {
                        // overflow action: clear buffer and continue (policy choice)
                        self.buf.clear();
                    }

                    match self.state {
                        ModemState::Idle => {
                            // detect command echo start (very simplistic)
                            if self.buf.starts_with("AT") && self.buf.ends_with("\r\n") {
                                self.state = ModemState::ReceivingResponse;
                                self.buf.clear();
                            } else if self.buf.ends_with("\r\n") {
                                // URC line — call URC handlers here (not implemented)
                                // Example: handle_urc(self.buf.trim());
                                self.last_response_or_urc_ms = now_ms();
                                self.buf.clear();
                            }
                        }

                        ModemState::ReceivingResponse => {
                            if b == b'\n' {
                                self.last_response_or_urc_ms = now_ms();

                                if self.buf.ends_with("OK\r\n") {
                                    self.ready = ModemReady::Ok;
                                } else if self.buf.ends_with("ERROR\r\n") {
                                    self.ready = ModemReady::Error;
                                } else if self.buf.ends_with("NO CARRIER\r\n") {
                                    self.ready = ModemReady::NoCarrier;
                                } else if self.buf.contains("CME ERROR") {
                                    self.ready = ModemReady::CmeError;
                                }

                                if self.ready != ModemReady::NotReady {
                                    self.state = ModemState::Idle;
                                    self.buf.clear();
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                Err(NbError::WouldBlock) => break,
                Err(NbError::Other(e)) => return Err(e),
            }
        }

        Ok(())
    }

    /// Blocking wait that repeatedly calls poll until a terminal response or timeout.
    pub fn wait_for_response_blocking<F>(
        &mut self,
        timeout_ms: u32,
        now_ms: &F,
    ) -> Result<ModemReady, E>
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

    /// Utility to issue a hard hardware reset (use only in emergency).
    pub fn hard_reset(&mut self) -> Result<(), E> {
        let _ = self.reset.set_low();
        // blocking long low pulse per datasheet
        // caller must provide their own delay function if needed (not included here)
        // This example uses a busy-wait placeholder: user should adapt to their HAL.
        // For safety we don't spin here since we don't have `now_ms` — caller can implement.
        Ok(())
    }
}
