// TO BE TESTED:
// 1. Battery Gauge
// 2. EEPROM
// 3. Temperature sensor
// 4. CAN
// 5. Network switches?

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};
use embassy_stm32::mode::Async;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, i2c, peripherals};

bind_interrupts!(struct Irqs {
    I2C4_ER => i2c::ErrorInterruptHandler<peripherals::I2C4>;
    I2C4_EV => i2c::EventInterruptHandler<peripherals::I2C4>;
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
});

const LTC_ADDR: u8 = 0b1100100;

const EEPROM_ADDR: u8 = 0x50;

#[embassy_executor::task]
async fn controller_task(mut con: i2c::I2c<'static, Async, i2c::Master>) {
    info!("Controller start");

    match con.write(LTC_ADDR, &[0x01, 0b10111100]).await {
        Ok(_) => {},
        Err(e) => error!("Failed to set LTC2944 configuration: {}", e),
    };

    loop {
        let mut read_data = [0u8; 2];

        match con.write_read(LTC_ADDR, &[0x0E], &mut read_data).await {
            Ok(_) => {
                let current_val: i32 = (((read_data[0] as u32) << 8) | (read_data[1] as u32)) as i32 - 32767;
                let current_val_f: f32 = (64.0 / 5.0) * (current_val as f32) / 32767.0;

                info!("current: {} ({})", current_val_f, read_data);
            }
            Err(e) => error!("Error writing {}", e),
        }

        match con.write_read(LTC_ADDR, &[0x08], &mut read_data).await {
            Ok(_) => {
                let voltage: u32 = ((read_data[0] as u32) << 8) | (read_data[1] as u32);
                let voltage_f = 70.8 * (voltage as f32) / 65535.0;

                info!("voltage: {} ({})", voltage_f, read_data);
            }
            Err(e) => error!("Error writing {}", e),
        }

        Timer::after_millis(1000).await;
    }
}

async fn startup_sequence(i2c_con: &mut i2c::I2c<'static, Async, i2c::Master>)
{
    let mut read_data = [0u8; 2];

    // 1. Read battery voltage and compare to EEPROM
    match i2c_con.write_read(LTC_ADDR, &[0x08], &mut read_data).await {
        Ok(_) => {
            let voltage: u32 = ((read_data[0] as u32) << 8) | (read_data[1] as u32);
            let voltage_f = 70.8 * (voltage as f32) / 65535.0;

            info!("Voltage: {} ({})", voltage_f, read_data);
        }
        Err(e) => error!("Failed to read voltage form LTC2944: {}", e),
    }

    match i2c_con.write_read(EEPROM_ADDR, &[0x00], &mut read_data).await {
        Ok(_) => {
            let voltage: u32 = ((read_data[0] as u32) << 8) | (read_data[1] as u32);
            let voltage_f = 70.8 * (voltage as f32) / 65535.0;

            info!("Voltage from EEPROM: {} ({})", voltage_f, read_data);
        }
        Err(e) => error!("Failed to read voltage from EEPROM: {}", e),
    }

    info!("Writing new voltage to EEPROM");
    match i2c_con.write(EEPROM_ADDR, &read_data).await {
        Ok(_) => {
            info!("Successfully wrote voltage data to EEPROM.");
        }
        Err(e) => error!("Failed to write voltage to EEPROM: {}", e),
    }

    // TODO:
    // 1. Read current voltage value
    // 2. Enable outputs
    // 3. Read voltage value from EEPROM
    // 4. Compare previous value to current:
    //   - If similar: take the charge value from EEPROM
    //   - If different: estimate charge from battery datasheet
    // 5. Write new voltage value and estimated charge to EEPROM
    // 6. Finish configuring LTC2944 with estimated charge
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    let mut status_led = Output::new(p.PC0, Level::Low, Speed::Low);

    // let mut led = Output::new(p.PC0, Level::Low, Speed::Low);
    let mut output_5v = Output::new(p.PC9, Level::Low, Speed::Low);
    let mut output_24v = Output::new(p.PA6, Level::Low, Speed::Low);

    let mut config = i2c::Config::default();
    config.frequency = Hertz::khz(400);

    let c_sda = p.PC7;
    let c_scl = p.PC6;
    let mut i2c_con = i2c::I2c::new(p.I2C4, c_scl, c_sda, p.DMA1_CH1, p.DMA1_CH2, Irqs, config);

    match i2c_con.write(LTC_ADDR, &[0x01, 0b10111100]).await {
        Ok(_) => {},
        Err(e) => error!("Failed to set LTC2944 configuration: {}", e),
    };

    startup_sequence(&mut i2c_con).await;

    // Enable outputs
    output_5v.set_high();
    output_24v.set_high();

    info!("5V status: {}", output_5v.get_output_level());
    info!("24V status: {}", output_24v.get_output_level());

    spawner.spawn(unwrap!(controller_task(i2c_con)));

    // TODO: tasks:
    // - CAN task: periodically send voltage, current and charge data on network
    // - LTC2944: regularly fetch the LTC2944 for voltage, current and charge data
    // - Switch fault ctl: wait for any fault on the two switches and general switch monitoring
    // - EEPROM task: regularly save the voltage and charge
    // - Temperature task: regulary check the temperature
    // - LED task: blinks the LED every second, stop blinking in case of fault (determined by CAN
    // task via signal)

    // EEPROM Task should write every 10s or when asked by the CAN task: use embassy-future:select
    // and signals for that.
    //
    // Switch fault CTL should wait for interrupts on the corresponding pins. It will share this
    // value to the CAN task using a Watch primitive.
    //
    // Temperature task should be run every 1/2 second and monitor any drastic temperature change.
    // Values should be sent to CAN task using a Watch primitive. Use embassy-future:select and
    // signals to automatically update the value.
    //
    // LTC2944 works similarly to EEPROM and temperature. If we have wired the alert pin, we should
    // also wait for interrupts in this task.
    //
    // For sharing i2c bus amongst devices, see:
    // - https://github.com/embassy-rs/embassy/blob/main/examples/rp/src/bin/shared_bus.rs
    // - https://docs.embassy.dev/embassy-embedded-hal/git/default/shared_bus/asynch/i2c/index.html

    // spawner.spawn(unwrap!(can_task(i2c_con)));
    // spawner.spawn(unwrap!(eeprom_task(i2c_con)));

    loop {
        status_led.toggle();
        Timer::after_millis(1000).await;
    }
}
