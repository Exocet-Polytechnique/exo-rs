//! # Description
//! Ds18b20 device driver to use with a 1-wire bus.
//!
//! See [`crate::one_wire_bus`] for important clock configuration details.
//!
//! # Examples
//! ## 1. Getting the address of a connected device
//! Use this if you need to determine the ROM code (address of a sensor). For the follwing example 
//! to work, only one sensor must be connected on the bus. Reading the address from multiple
//! devices on the same board has not been implemented.
//! ```
//! let p = embassy_stm32::init(get_device_config())
//!
//! let mut bus = OneWireBus::init(p.PC3);
//!
//! match bus.read_rom() {
//!     Err(e) => info!("Error reading sensor value: {}", e),
//!     Ok(addr) => info!("Found sensor with address {:x}", addr),
//! }
//! ```
//! It is not recommended to fetch the address of the device at the start of the program. Instead,
//! it should be noted down and added directly into the code (either hard-coded or with a
//! compile-time configuration). Addresses are only useful when multiple devices are sensors are
//! connected on the same bus. See second example below.
//!
//! ## 2. Reading the temperature from a sensor
//! In the following example, we initialize the bus and the sensor, configure the sensor and read
//! the temperature roughly every second (since the conversion time increases the delay in each
//! iteration).
//! ```
//! let p = embassy_stm32::init(get_device_config());
//!
//! let mut bus = OneWireBus::init(p.PC3);
//! let mut sensor = DS18B20::new(None);
//! sensor.ensure_config(&mut bus, ds18b20::Config::new(ds18b20::Resolution::NineBits)).unwrap();
//!
//! loop {
//!     match sensor.read_temperature(&mut bus).await {
//!         Err(e) => info!("Failed to read temperature: {}", e),
//!         Ok(temperature) => info!("Temperature: {} C", temperature),
//!     }
//!     Timer::after_millis(1000).await;
//! }
//! ```
//! Here, it is assumed that there is only one 1-wire device on the bus. Therefore, the address
//! given to the temperature sensor is `None`. If more than one device shared the 1-wire bus,
//! multiple sensors objects must be created, each with their address passed as argument to `new`
//! (instead of `None`). The address does not need to include the CRC-8 code.

use crc::CRC_8_MAXIM_DOW;
use embassy_time::Timer;

use crate::ds2484::{DS2484, Error};

const BASE_CONFIGURATION_REG: u8 = 0x1F; // see docs
const CONFIG_RESOLUTION_BITSHIFT: u8 = 5;

const NINE_BITS_CONVERSION_DELAY_MS: u64 = 100;
const TEN_BITS_CONVERSION_DELAY_MS: u64 = 200;
const ELEVEN_BITS_CONVERSION_DELAY_MS: u64 = 400;
const TWELVE_BITS_CONVERSION_DELAY_MS: u64 = 800;

#[repr(u8)]
enum FunctionCommands {
    Convert = 0x44,
    WriteScratchpad = 0x4E,
    ReadScratchpad = 0xBE,
    CopyScratchpad = 0x48,
    RecallEEPROM = 0xB8,
}

#[derive(Clone, Copy)]
pub enum Resolution {
    NineBits = 0b00,
    TenBits = 0b01,
    ElevenBits = 0b10,
    TwelveBits = 0b11,
}

pub struct Config {
    pub resolution: Resolution,
}

impl Config {
    pub fn new(resolution: Resolution) -> Self {
        Self { resolution }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            resolution: Resolution::NineBits,
        }
    }
}

// #[derive(Debug, defmt::Format)]
// pub enum Error {
//     InvalidCrc,
//     BusError(one_wire_bus::Error),
//     UnexpectedReservedValue,
// }

// impl From<one_wire_bus::Error> for Error {
//     fn from(value: one_wire_bus::Error) -> Self {
//         Self::BusError(value)
//     }
// }

pub struct DS18B20 {
    address: Option<u64>,
    config: Option<Config>,
}

impl DS18B20 {
    /// Create a new DS18B20 device with a given address.
    /// > **Note**: set address to None if _and only if_ there is no other device on the 1-wire
    /// > bus.
    pub fn new(address: Option<u64>) -> Self {
        Self {
            address,
            config: None,
       }
    }

    async fn read_scratchpad(&mut self, bus: &mut DS2484) -> Result<(f32, [u8; 3]), Error> {
        let mut scratchpad_data = [0; 9];
        bus.send_command_read(
            FunctionCommands::ReadScratchpad as u8,
            self.address,
            &mut scratchpad_data,
        ).await?;

        let calculated_crc = crc::Crc::<u8>::new(&CRC_8_MAXIM_DOW).checksum(&scratchpad_data[..8]);

        if scratchpad_data[8] != calculated_crc {
            return Err(Error::InvalidCrc);
        }

        if scratchpad_data[8] != calculated_crc {
            defmt::error!("CRC mismatch: got 0x{:02X} expected 0x{:02X}", scratchpad_data[8], calculated_crc);
            return Err(Error::InvalidCrc);
        }

        // bit-shift manipulations could change the sign of the value, therefore we manipulate
        // unsigned values first
        let temperature = (scratchpad_data[0] as u16) | ((scratchpad_data[1] as u16) << 8);
        // then we can take the sign into account
        let temperature = temperature as i16;
        // Convert fixed-point value (as i16) to floating point. Notes:
        // 1. We ignore the fact that the last bits are undefined for precisions below 12bits. This
        //    is fine since they only cause small variations in the output.
        // 2. The value 16.0 comes from the fact that the decimal point is between the 3rd and 4th
        //    bit (2 ^ 4 = 16).
        let temperature = (temperature as f32) / 16.0;

        // unwrap: we hard-code the slice to be 3-byte long, and the scratch_pad data array is
        // 9 bits long.
        let configuration_bytes = scratchpad_data[2..=4].try_into().unwrap();

        Ok((temperature, configuration_bytes))
    }

    /// Write the given configuration on the RAM and persistent memories of the sensor.
    /// Will not write or change the memory if the correct configuration is already loaded.
    pub async fn ensure_config(&mut self, bus: &mut DS2484, config: Config) -> Result<(), Error> {
        bus.send_command(FunctionCommands::RecallEEPROM as u8, self.address).await?;

        // wait for EEPROM recall to complete
        Timer::after_millis(10).await;

        let config_byte =
            BASE_CONFIGURATION_REG | ((config.resolution as u8) << CONFIG_RESOLUTION_BITSHIFT);
        let expected_data = [0xFF, 0x00, config_byte];

        let scratchpad_data = self.read_scratchpad(bus).await?;

        if scratchpad_data.1 == expected_data {
            self.config = Some(config);
            return Ok(());
        }

        bus.send_command_with_data(
            FunctionCommands::WriteScratchpad as u8,
            self.address,
            &expected_data,
        ).await?;

        // Wait before copy
        Timer::after_millis(5).await;

        bus.send_command(FunctionCommands::CopyScratchpad as u8, self.address).await?;

        // wait for EEPROM recall to complete
        Timer::after_millis(10).await;

        self.config = Some(config);

        Ok(())
    }

    /// Get the temperature from the sensor, which takes a varying amount of time depending on the
    /// precision required in the config. The function is async to allow other tasks to execute
    /// while the temperature is being measured and converted on the sensor.
    ///
    /// **Important**: make sure the configuration is set by calling [`Self::ensure_config`] before
    /// calling this method.
    pub async fn read_temperature(&mut self, bus: &mut DS2484) -> Result<f32, Error> {
        bus.send_command(FunctionCommands::Convert as u8, self.address).await?;
        let config = self.config.as_ref().expect("Configuration should be set using `DS18B20::ensure_config(...)` before reading temperature");

        let delay = match config.resolution {
            Resolution::NineBits => NINE_BITS_CONVERSION_DELAY_MS,
            Resolution::TenBits => TEN_BITS_CONVERSION_DELAY_MS,
            Resolution::ElevenBits => ELEVEN_BITS_CONVERSION_DELAY_MS,
            Resolution::TwelveBits => TWELVE_BITS_CONVERSION_DELAY_MS,
        };

        Timer::after_millis(delay).await;

        let (temperature, _) = self.read_scratchpad(bus).await?;

        Ok(temperature)
    }
}