#![no_std]
#![no_main]

use defmt::{info};
use embassy_executor::Spawner;
use embassy_time::Timer;
use embassy_stm32::{
    Config,
    i2c::{self, I2c, Config as I2cConfig},
    time::{Hertz},
    bind_interrupts,
    peripherals
};
use exo_drivers::{
    ds2484::{DS2484},
    ds18b20::{self, DS18B20}
};
use {defmt_rtt as _, panic_probe as _};

fn get_device_config() -> Config {
    let mut config = Config::default();

    {
        use embassy_stm32::rcc::*;
        config.rcc.hsi = true;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV2,
            mul: PllMul::MUL25,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV2),
        });
        config.rcc.sys = Sysclk::PLL1_R;
    }

    config
}

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {

    let p = embassy_stm32::init(get_device_config());

    let i2c = I2c::new(
        p.I2C1,
        p.PB8,                  // I2C1_SCL pin
        p.PB7,                  // I2C1_SDA pin
        p.DMA1_CH6,           // TX DMA channel
        p.DMA1_CH7,           // RX DMA channel
        Irqs,                   // Interrupt bindings
        I2cConfig::default(),
    );

    let mut bus = DS2484::new(i2c);

    let address = match bus.read_rom().await {
        Ok(addr) => {
            defmt::info!("found DS18B20 at address: {:?}", addr);
            addr
        }
        Err(e) => {
            defmt::error!("error: {:?}", e);
            panic!("cannot continue without device");
        }
    };

    let mut sensor = DS18B20::new(Some(address));

    match sensor.ensure_config(
        &mut bus,
        ds18b20::Config::new(ds18b20::Resolution::TwelveBits),
    ).await {
        Ok(())  => defmt::info!("config set"),
        Err(e)  => panic!("ensure_config failed: {:?}", e),
    }

    // give the 1-Wire bus time to recover after read_rom
    Timer::after_millis(100).await;

    loop {
        match sensor.read_temperature(&mut bus).await {
            Err(e) => info!("Failed to read temperature: {}", e),
            Ok(temperature) => info!("Temperature: {} C", temperature),
        }
        Timer::after_millis(1000).await;
    }
}