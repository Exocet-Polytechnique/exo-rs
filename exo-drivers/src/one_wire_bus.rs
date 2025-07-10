//! # Description
//! 1-Wire bus on which one or more sensors can be connected
//!
//! > **Note**: The protocol was implemented by bit-banging. Therefore, it is required that the
//! > system and GPIO peripheral clocks have a good enough resolution (some delays are as small as
//! > 1us). A clock speed of 100 MHz is a good value.
//!
//! # References:
//! - [DS18B20 datasheet](https://www.analog.com/media/en/technical-documentation/data-sheets/DS18B20.pdf)
//! - [Book of iButton(r) Standards](https://www.analog.com/media/en/technical-documentation/tech-articles/book-of-ibuttonreg-standards.pdf)
//! - [Interfacing the DS18X20/DS1822 1-Wire(r) Temperature Sensor in a Microcontroller Environment ](https://www.analog.com/en/resources/technical-articles/interfacing-the-ds18x20ds1822-1wire-temperature-sensor-in-a-microcontroller-environment.html)
//!
//! # Example
//! See [`crate::ds18b20`]

use core::result::Result::*;

use crc::CRC_8_MAXIM_DOW;

use embassy_stm32::{
    Peripheral,
    gpio::{Flex, Pin, Speed},
};

use embassy_time::{block_for, Duration, Instant, Timer};

/// t_RSTL
const RESET_LOW_DURATION: Duration = Duration::from_micros(480);
/// t_RSTH
const RESET_HIGH_DURATION: Duration = Duration::from_micros(480);
/// t_LOW0
const WRITE_ZERO_PULSE_DURATION: Duration = Duration::from_micros(60);
/// t_LOW1
const WRITE_ONE_PULSE_DURATION: Duration = Duration::from_micros(15);
/// t_REC
const RECOVERY_DURATION: Duration = Duration::from_micros(2);
/// t_SLOT
const SLOT_DURATION: Duration = Duration::from_micros(60);
/// duration of low pulse to tell the sensors the leader is ready to read
const READ_INIT_PULSE_DURATION: Duration = Duration::from_micros(2);
/// t_RDV
const READ_DATA_VALID_DURATION: Duration = Duration::from_micros(11);

/// 1-wire bus handle
pub struct OneWireBus {
    gpio: Flex<'static>,
}

/// Possible 1-wire ROM commands (network layer)
#[repr(u8)]
enum RomCommand {
    ReadRom = 0x33,
    MatchRom = 0x55,
    SkipRom = 0xCC,
}

/// possible 1-wire bus errors
#[derive(Debug, defmt::Format)]
pub enum Error {
    NoDeviceOnReset,
    NoDeviceOnSearch,
    TooManyDevices,
    NotEnoughDevices,
    InvalidCrc,
}

impl OneWireBus {
    /// Create a new 1-wire bus handle
    pub fn init(pin: impl Peripheral<P = impl Pin> + 'static) -> Self {
        let mut gpio = Flex::new(pin);
        gpio.set_as_input_output(Speed::Low);

        Self { gpio }
    }

    /// Send a reset pulse and check if a device responds
    async fn reset_and_detect(&mut self) -> Result<(), Error> {
        // Send reset pulse
        self.gpio.set_low();
        Timer::after(RESET_LOW_DURATION).await;
        self.gpio.set_high();

        // Wait until end of reset sequence
        let reset_end_instant = Instant::now() + RESET_HIGH_DURATION;
        let mut is_found = false;
        while Instant::now() < reset_end_instant {
            // Check if there is a sensor connected on the bus.
            // According to the datasheet, this should happen within 15 to 300 ms
            // after the end of the reset pulse, but we'll ignore timing here since
            // it doesn't really matter.
            if !is_found && self.gpio.is_low() {
                is_found = true;
            }

            // The loop will last for `RESET_HIGH_DURATION` no matter if a sensor
            // is found or not.
        }

        if is_found {
            Ok(())
        } else {
            Err(Error::NoDeviceOnReset)
        }
    }

    /// Write a single bit on the bus
    async fn write_bit(&mut self, value: bool) -> () {
        self.gpio.set_low();
        let write_end_instant = Instant::now() + SLOT_DURATION;

        if value {
            block_for(WRITE_ONE_PULSE_DURATION);
        } else {
            block_for(WRITE_ZERO_PULSE_DURATION);
        }

        self.gpio.set_high();

        while Instant::now() < write_end_instant {}

        Timer::after(RECOVERY_DURATION).await;
    }

    /// Read a single bit from the bus.
    /// Note: a value of 1 (true) could also mean no sensor is connected
    async fn read_bit(&mut self) -> bool {
        // Read trigger pulse
        self.gpio.set_low();
        let read_end_instant = Instant::now() + SLOT_DURATION;
        block_for(READ_INIT_PULSE_DURATION);
        self.gpio.set_high();

        block_for(READ_DATA_VALID_DURATION - READ_INIT_PULSE_DURATION);
        let value = self.gpio.is_high();

        while Instant::now() < read_end_instant {}

        Timer::after(RECOVERY_DURATION).await;

        value
    }

    /// Write a byte on the 1-wire bus
    async fn write_byte(&mut self, byte: u8) -> () {
        for i in 0..8 {
            self.write_bit((byte >> i) & 0x01 == 0x01).await;
        }
    }

    /// Read a byte from the 1-wire bus
    async fn read_byte(&mut self) -> u8 {
        let mut data: u8 = 0;
        for i in 0..8 {
            data |= (self.read_bit().await as u8) << i;
        }

        data
    }

    /// Transmit a rom command on the bus
    async fn send_rom_command(&mut self, command: RomCommand) -> () {
        self.write_byte(command as u8).await;
    }

    /// Fetch the address from a 1-wire device. IMPORTANT: there must be only one device on the
    /// bus.
    pub async fn read_rom(&mut self) -> Result<u64, Error> {
        self.reset_and_detect().await?;
        self.send_rom_command(RomCommand::ReadRom).await;

        // the first 7 bytes make up the address
        let mut address: u64 = 0;
        for i in 0..7 {
            let data = self.read_byte().await;
            address |= (data as u64) << (i * 8);
        }

        // and the last byte is the crc-8 code
        let received_crc = self.read_byte().await;
        let calculated_crc =
            crc::Crc::<u8>::new(&CRC_8_MAXIM_DOW).checksum(&address.to_le_bytes()[..7]);
        if received_crc != calculated_crc {
            return Err(Error::InvalidCrc);
        }

        Ok(address)
    }

    /// Send a function command
    pub async fn send_command(&mut self, command: u8, address: Option<u64>) -> Result<(), Error> {
        self.reset_and_detect().await?;
        if let Some(address) = address {
            self.send_rom_command(RomCommand::MatchRom).await;
            for i in 0..7 {
                self.write_byte(((address >> (i * 8)) & 0xFF) as u8).await;
            }

            self.write_byte(
                crc::Crc::<u8>::new(&CRC_8_MAXIM_DOW).checksum(&address.to_le_bytes()[..7]),
            ).await;
        } else {
            self.send_rom_command(RomCommand::SkipRom).await;
        }

        self.write_byte(command).await;

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
            self.write_byte(*byte).await;
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
            out_data[i] = self.read_byte().await;
        }

        Ok(())
    }
}
