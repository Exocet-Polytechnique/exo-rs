#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::Config;
use embassy_time::Timer;
use exo_drivers::{
    ds18b20::{self, DS18B20},
    one_wire_bus::OneWireBus,
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

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(get_device_config());

    let mut bus = OneWireBus::init(p.PC3);

    let address = bus.read_rom().await.unwrap();
    info!("Device with address: 0x{:014x} found!", address);

    // Create a device with no address: must be the only device on the bus
    let mut sensor = DS18B20::new(None);

    sensor
        .ensure_config(
            &mut bus,
            ds18b20::Config::new(ds18b20::Resolution::NineBits),
        )
        .await
        .unwrap();

    loop {
        match sensor.read_temperature(&mut bus).await {
            Err(e) => info!("Failed to read temperature: {}", e),
            Ok(temperature) => info!("Temperature: {} C", temperature),
        }
        //Timer::after_millis(1000).await;
    }
}
