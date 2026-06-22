use crc::CRC_8_MAXIM_DOW;

use embassy_stm32::{
    i2c::{self, I2c, Master},
    mode::Async
};
use embassy_time::Timer;

const DS2484_ADDR: u8 = 0x18;   


// DS2484 Commands
const COMMAND_1WWB: u8 = 0xA5;
const COMMAND_1WRB: u8 = 0x96;
const COMMAND_SRP: u8 = 0xE1;
const COMMAND_READ_DATA_REG: u8 = 0xE1;
const COMMAND_STATUS_REG: u8 = 0xF0;
const COMMAND_1WIRE_RESET: u8 = 0xB4;
const COMMAND_1WT: u8 = 0x78;

pub struct DS2484 {
    i2c: I2c<'static, Async, Master>,
}

/// Possible 1-wire ROM commands (network layer)
#[repr(u8)]
enum RomCommand {
    ReadRom = 0x33,
    MatchRom = 0x55,
    SkipRom = 0xCC,
    SearchRom = 0xF0,
}

/// possible DS2484 bus errors
#[derive(Debug, defmt::Format)]
pub enum Error {
    I2cError(i2c::Error),
    NoDeviceOnReset,
    InvalidCrc,
    UnexpectedReservedValue
}

impl From<embassy_stm32::i2c::Error> for Error {
    fn from(e: i2c::Error) -> Self {
        Error::I2cError(e)
    }
}

impl DS2484 {
    /// Constructor
    pub fn new(i2c: I2c<'static, Async, Master>) -> Self {
        Self { i2c }
    }

    /// Send a reset pulse and check if a device responds
    async fn reset_and_detect(&mut self) -> Result<(), Error> {
        // Send 1-Wire Reset command to DS2484 over I2C
        self.i2c.write(DS2484_ADDR, &[COMMAND_1WIRE_RESET]).await?;

        // give DS2484 time to start the reset sequence before polling
        Timer::after_micros(500).await;

        // Wait for DS2484 to complete the reset sequence
        self.wait_1wire_idle().await?;

        // Read status register to check Presence Pulse Detected (PPD) bit
        let set_pointer = [COMMAND_SRP, COMMAND_STATUS_REG];
        let mut status = [0u8; 1];
        self.i2c
            .write_read(DS2484_ADDR, &set_pointer, &mut status)
            .await?;

        // bit 1 of status register = PPD (Presence Pulse Detected)
        let device_found = (status[0] & 0x02) != 0;

        if device_found {
            Ok(())
        } else {
            Err(Error::NoDeviceOnReset)
        }
    }

    // Check if the 1-wire bus is idle (unused)
    async fn is_1wire_free(&mut self) -> Result<bool, Error> {
        let set_pointer: [u8; 2] = [COMMAND_SRP, COMMAND_STATUS_REG];
        let mut status: [u8; 1] = [0u8; 1];
        self.i2c.write_read(DS2484_ADDR, &set_pointer, &mut status).await?;

        let is_free: bool = (status[0] & 0x01) == 0; // bit 0 (1-Wire idle)
        Ok(is_free)
    }

    // Wait until the 1-wire bus is idle (unused)
    async fn wait_1wire_idle(&mut self) -> Result<(), Error> {
        loop {
            if self.is_1wire_free().await? {
                return Ok(())
            }
            embassy_time::Timer::after_micros(100).await;
        }
    }

    /// Write a byte on the 1-wire bus
    async fn write_byte(&mut self, byte: u8) -> Result<(), Error> {
        self.wait_1wire_idle().await?;
        self.i2c.write(DS2484_ADDR, &[COMMAND_1WWB, byte]).await?;
        Timer::after_micros(100).await; // give DS2484 time to start transmitting
        Ok(())
    }

    /// Read a byte from the 1-wire bus
    async fn read_byte(&mut self) -> Result<u8, Error> {
        self.wait_1wire_idle().await?;
        self.i2c.write(DS2484_ADDR, &[COMMAND_1WRB]).await?;
        self.wait_1wire_idle().await?;
        let set_pointer: [u8; 2] = [COMMAND_SRP, COMMAND_READ_DATA_REG];
        let mut buffer: [u8; 1] = [0u8; 1];
        self.i2c.write_read(DS2484_ADDR, &set_pointer, &mut buffer).await?;
        Ok(buffer[0])
    }

    /// Transmit a rom command on the bus
    async fn send_rom_command(&mut self, command: RomCommand) -> Result<(), Error> {
        self.write_byte(command as u8).await?;
        Ok(())
    }

    /// Fetch the address from a 1-wire device. IMPORTANT: there must be only one device on the
    /// bus.
    pub async fn read_rom(&mut self) -> Result<u64, Error> {
        self.reset_and_detect().await?;
        self.send_rom_command(RomCommand::ReadRom).await?;

        let mut rom = [0u8; 8];
        for i in 0..8 {  // 8 bytes total (7 address + 1 CRC)
            rom[i] = self.read_byte().await?;
        }

        // CRC is calculated over first 7 bytes
        let calculated_crc = crc::Crc::<u8>::new(&CRC_8_MAXIM_DOW)
            .checksum(&rom[..7]);  // check against rom[..7]
        if rom[7] != calculated_crc {
            return Err(Error::InvalidCrc);
        }

        let mut address: u64 = 0;
        for i in 0..8 { // changed to 8 bytes (including CRC)
            address |= (rom[i] as u64) << (i * 8);
        }

        Ok(address)
    }

    /// Send a function command
    pub async fn send_command(&mut self, command: u8, address: Option<u64>) -> Result<(), Error> {
        self.reset_and_detect().await?;
        if let Some(address) = address {
            self.send_rom_command(RomCommand::MatchRom).await?;
            for i in 0..8 {
                self.write_byte(((address >> (i * 8)) & 0xFF) as u8).await?;
            }
        } else {
            self.send_rom_command(RomCommand::SkipRom).await?;
        }

        self.write_byte(command).await?;

        Ok(())
    }

    /// Send a function command, followed by data bytes
    pub async fn send_command_with_data(
        &mut self,
        command: u8,
        address: Option<u64>,
        data: &[u8],
    ) -> Result<(), Error> {
        self.send_command(command, address).await?;

        for byte in data {
            self.write_byte(*byte).await?;
        }

        Ok(())
    }

    /// Send a function command and read bytes from device
    pub async fn send_command_read(
        &mut self,
        command: u8,
        address: Option<u64>,
        out_data: &mut [u8],
    ) -> Result<(), Error> {
        self.send_command(command, address).await?;

        for i in 0..out_data.len() {
            out_data[i] = self.read_byte().await?;
        }

        Ok(())
    }
}