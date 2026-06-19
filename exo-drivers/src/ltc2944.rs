use defmt::*;

use embassy_stm32::i2c;
use embassy_stm32::mode::Async;

const I2C_ADDRESS: u8 = 0x64;

const CONTROL_REG_ADDRESS: u8 = 0x01;

const ADC_MODE_CONFIG_BITSHIFT: usize = 6;
const ADC_MODE_CONFIG_MASK: u8 = 0b11000000;
const PRESCALER_CONFIG_BITSHIFT: usize = 3;
const PRESCALER_CONFIG_MASK: u8 = 0b00111000;
const NALCC_CONFIG_BITSHIFT: usize = 1;
const NALCC_CONFIG_MASK: u8 = 0b00000110;
const SHUTDOWN_CONFIG_MASK: u8 = 0b00000001;

#[repr(u8)]
pub enum ADCMode {
    Sleep = 0,
    Single = 1,
    Scan = 2,
    Automatic = 3,
}

impl Default for ADCMode {
    fn default() -> Self {
        Self::Sleep
    }
}

impl ADCMode {
    fn from_config_byte(byte: u8) -> Self {
        let adc_mode_bits = (byte & ADC_MODE_CONFIG_MASK) >> ADC_MODE_CONFIG_BITSHIFT;

        match adc_mode_bits {
            0b00 => Self::Sleep,
            0b01 => Self::Single,
            0b10 => Self::Scan,
            0b11 => Self::Automatic,
            _ => defmt::unreachable!(),
        }
    }
}

/// Coulomb counter prescale value
#[repr(u8)]
pub enum Prescaler {
    Prescale1 = 0,
    Prescale4 = 1,
    Prescale16 = 2,
    Prescale64 = 3,
    Prescale256 = 4,
    Prescale1024 = 5,
    Prescale4096 = 7,
}

impl Default for Prescaler {
    fn default() -> Self {
        Self::Prescale4096
    }
}

impl Prescaler {
    fn from_config_byte(byte: u8) -> Self {
        let prescaler_bits = (byte & PRESCALER_CONFIG_MASK) >> PRESCALER_CONFIG_BITSHIFT;

        match prescaler_bits {
            0b000 => Self::Prescale1,
            0b001 => Self::Prescale4,
            0b010 => Self::Prescale16,
            0b011 => Self::Prescale64,
            0b100 => Self::Prescale256,
            0b101 => Self::Prescale1024,
            0b110 | 0b111 => Self::Prescale4096,
            _ => defmt::unreachable!(),
        }
    }
}

#[repr(u8)]
pub enum NALCCConfiguration {
    Alert = 2,
    ChargeComplete = 1,
    Disabled = 0,
}

impl Default for NALCCConfiguration {
    fn default() -> Self {
        Self::Alert
    }
}

impl NALCCConfiguration {
    fn from_config_byte(byte: u8) -> Self {
        let nalcc_config_bits = (byte & NALCC_CONFIG_MASK) >> NALCC_CONFIG_BITSHIFT;

        match nalcc_config_bits {
            0b00 => Self::Disabled,
            0b01 => Self::ChargeComplete,
            0b10 => Self::Alert,
            _ => defmt::unreachable!(),
        }
    }
}

pub struct Configuration {
    pub adc_mode: ADCMode,
    pub prescaler: Prescaler,
    pub nalcc_configuration: NALCCConfiguration,
    pub shutdown: bool,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            adc_mode: ADCMode::default(),
            prescaler: Prescaler::default(),
            nalcc_configuration: NALCCConfiguration::default(),
            shutdown: false,
        }
    }
}

impl Configuration {
    fn to_byte(self: Configuration) -> u8 {
        ((self.adc_mode as u8) << ADC_MODE_CONFIG_BITSHIFT)
            | ((self.prescaler as u8) << PRESCALER_CONFIG_BITSHIFT)
            | ((self.nalcc_configuration as u8) << NALCC_CONFIG_BITSHIFT)
            | self.shutdown
    }

    fn from_byte(byte: u8) -> Self {
        let adc_mode = ADCMode::from_config_byte(byte);
        let prescaler = Prescaler::from_config_byte(byte);
        let nalcc_configuration = NALCCConfiguration::from_config_byte(byte);
        let shutdown = (byte & SHUTDOWN_CONFIG_MASK) != 0;

        Self {
            adc_mode,
            prescaler,
            nalcc_configuration,
            shutdown,
        }
    }
}

pub struct LTC2944 {
    r_sense: f32,
}

type I2CBus = i2c::I2c<'static, Async, i2c::Master>;

impl LTC2944 {
    pub async fn new(mut i2c_bus: I2CBus, configuration: Option<Configuration>, r_sense: f32) -> Self {
        let ltc2944 = Self {
            r_sense
        };

        match ltc2944
            .write_configuration(i2c_bus, configuration.unwrap_or_default())
            .await
        {
            Ok(_) => {}
            Err(e) => error!("Failed to set LTC2944 configuration: {}", e),
        };

        ltc2944
    }

    async fn write_configuration(
        self: &Self,
        mut i2c_bus: I2CBus,
        configuration: Configuration,
    ) -> Result<(), i2c::Error> {
        let _ = self;
        let control_byte = configuration.to_byte();

        i2c_bus
            .write(I2C_ADDRESS, &[CONTROL_REG_ADDRESS, control_byte])
            .await
    }
}
