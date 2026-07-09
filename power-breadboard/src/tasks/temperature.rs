use embassy_stm32::{
    i2c::{I2c, Master},
    mode::Async,
};
use exo_drivers::{
    ds18b20::{self, DS18B20},
    ds2484::DS2484,
};

use crate::{
    tasks::can::{CanData, DATA_CHANNEL, ERROR_CHANNEL, ErrorType, WARNING_CHANNEL, WarningType},
};

const WARNING_THRESHOLD: f32 = 42.0;
const ERROR_THRESHOLD: f32 = 60.0;

#[embassy_executor::task]
pub async fn temperature_task(i2c_bus: I2c<'static, Async, Master>) {
    let mut ds2484 = DS2484::new(i2c_bus);
    let mut sensor = DS18B20::new(None); // Only one sensor so address not required

    if sensor
        .ensure_config(
            &mut ds2484,
            ds18b20::Config::new(ds18b20::Resolution::NineBits),
        )
        .await
        .is_err()
    {
        ERROR_CHANNEL.try_send(ErrorType::AuxBattTempSensorMissing).unwrap();
        return;
    }

    let mut iter_count = 0;
    let mut fail_count = 0;

    loop {
        // Reading a value takes about 100 ms
        if let Ok(temperature) = sensor.read_temperature(&mut ds2484).await {
            fail_count = 0;

            if iter_count == 0 {
                _ = DATA_CHANNEL.try_send(CanData::Temperature(temperature));
            }

            if temperature >= ERROR_THRESHOLD {
                _ = ERROR_CHANNEL.try_send(ErrorType::AuxBattTempTooHigh);
            } else if temperature >= WARNING_THRESHOLD {
                _ = WARNING_CHANNEL.try_send(WarningType::AuxBattTempHigh);
            }

        } else {
            fail_count += 1;
            if fail_count >= 10 {
                fail_count = 0;
                _ = ERROR_CHANNEL.try_send(ErrorType::AuxBattTempSensorDisconnected);
            }
        }

        iter_count += 1;
        iter_count %= 5;
    }
}
