// TO BE TESTED:
// 1. Battery Gauge
// 2. EEPROM
// 3. Temperature sensor
// 4. CAN
// 5. Network switches?

#![no_std]
#![no_main]

mod battery_soc;
mod can_frames;
mod fault;

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::can;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::watch::Watch;
use embassy_time::Timer;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};
use embassy_stm32::mode::Async;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, i2c, peripherals};
use exo_drivers::ds18b20::{self, DS18B20};
use exo_drivers::ltc2944::{self, LTC2944};
use exo_drivers::one_wire_bus::OneWireBus;
use fault::Latch;

bind_interrupts!(struct Irqs {
    I2C4_ER => i2c::ErrorInterruptHandler<peripherals::I2C4>;
    I2C4_EV => i2c::EventInterruptHandler<peripherals::I2C4>;
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
    FDCAN1_IT0 => can::IT0InterruptHandler<peripherals::FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<peripherals::FDCAN1>;
});

const LTC_ADDR: u8 = 0b1100100;

const EEPROM_ADDR: u8 = 0x50;

/// Current sense resistor value used by the LTC2944 current formula, in milliohms.
const LTC_R_SENSE_MOHM: f32 = 5.0;

const CAN_BITRATE: u32 = 250_000;

/// Battery temperature thresholds, in Celsius.
const TEMPERATURE_WARNING_THRESHOLD_C: f32 = 45.5;
const TEMPERATURE_ERROR_THRESHOLD_C: f32 = 58.5;

/// Battery state-of-charge thresholds, in percent.
const CHARGE_WARNING_THRESHOLD_PERCENT: f32 = 30.0;
const CHARGE_ERROR_THRESHOLD_PERCENT: f32 = 15.0;

type BatteryWatch = Watch<CriticalSectionRawMutex, BatteryReading, 1>;

static BATTERY_WATCH: BatteryWatch = Watch::new();

#[derive(Clone, Copy, Default)]
struct BatteryReading {
    voltage_v: f32,
    current_a: f32,
    soc_percent: f32,
}

/// Every 100ms: read the battery voltage/current, estimate the state of charge, and report it to
/// [`BATTERY_WATCH`] for [`battery_data_task`] to broadcast at a slower rate. Warns/errors on low
/// charge (edge-triggered, so it doesn't spam the bus).
#[embassy_executor::task]
async fn battery_task(
    mut i2c_bus: i2c::I2c<'static, Async, i2c::Master>,
    ltc: LTC2944,
    mut can_tx: can::BufferedCanSender,
) {
    info!("Battery task start");

    let sender = BATTERY_WATCH.sender();

    let mut low_charge_warning = Latch::new();
    let mut low_charge_error = Latch::new();

    loop {
        match (ltc.read_voltage(&mut i2c_bus).await, ltc.read_current(&mut i2c_bus).await) {
            (Ok(voltage_v), Ok(current_a)) => {
                let soc_percent = battery_soc::estimate_soc_percent(voltage_v);

                info!("Battery: {}V, {}A, {}% charge", voltage_v, current_a, soc_percent);

                sender.send(BatteryReading { voltage_v, current_a, soc_percent });

                let warning_triggered =
                    low_charge_warning.rising_edge(soc_percent < CHARGE_WARNING_THRESHOLD_PERCENT);
                let error_triggered =
                    low_charge_error.rising_edge(soc_percent < CHARGE_ERROR_THRESHOLD_PERCENT);

                if error_triggered {
                    can_tx.write(can_frames::error_frame(can_frames::ErrorType::BatteryFault)).await;
                }
                if warning_triggered {
                    can_tx.write(can_frames::warning_frame(can_frames::WarningType::LowCharge)).await;
                }
            }
            (voltage, current) => {
                error!("Failed to read battery voltage/current from LTC2944: voltage={}, current={}", voltage.is_err(), current.is_err());
            }
        }

        Timer::after_millis(100).await;
    }
}

/// Every 1s: broadcast the latest voltage/current/charge values measured by [`battery_task`].
#[embassy_executor::task]
async fn battery_data_task(mut can_tx: can::BufferedCanSender) {
    info!("Battery data task start");

    // unwrap: we are the only receiver ever created from `BATTERY_WATCH`.
    let mut receiver = unwrap!(BATTERY_WATCH.receiver());

    loop {
        let reading = receiver.get().await;

        can_tx
            .write(can_frames::battery_data_frame(reading.soc_percent, reading.current_a, reading.voltage_v))
            .await;

        Timer::after_millis(1000).await;
    }
}

/// Every 500ms: read the battery temperature and broadcast it. Warns/errors on overtemperature
/// (edge-triggered, so it doesn't spam the bus).
#[embassy_executor::task]
async fn temperature_task(mut bus: OneWireBus, mut sensor: DS18B20, mut can_tx: can::BufferedCanSender) {
    info!("Temperature task start");

    let mut overtemperature_warning = Latch::new();
    let mut overtemperature_error = Latch::new();

    loop {
        match sensor.read_temperature(&mut bus).await {
            Ok(temperature_c) => {
                info!("Battery temperature: {}C", temperature_c);

                can_tx.write(can_frames::temperature_data_frame(temperature_c)).await;

                let warning_triggered =
                    overtemperature_warning.rising_edge(temperature_c >= TEMPERATURE_WARNING_THRESHOLD_C);
                let error_triggered =
                    overtemperature_error.rising_edge(temperature_c >= TEMPERATURE_ERROR_THRESHOLD_C);

                if error_triggered {
                    can_tx.write(can_frames::error_frame(can_frames::ErrorType::Overtemperature)).await;
                }
                if warning_triggered {
                    can_tx.write(can_frames::warning_frame(can_frames::WarningType::TemperatureWarning)).await;
                }
            }
            Err(e) => error!("Failed to read battery temperature: {}", e),
        }

        Timer::after_millis(500).await;
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

    let mut output_5v = Output::new(p.PC9, Level::Low, Speed::Low);
    let mut output_24v = Output::new(p.PA6, Level::Low, Speed::Low);

    let mut config = i2c::Config::default();
    config.frequency = Hertz::khz(400);

    let c_sda = p.PC7;
    let c_scl = p.PC6;
    let mut i2c_con = i2c::I2c::new(p.I2C4, c_scl, c_sda, p.DMA1_CH1, p.DMA1_CH2, Irqs, config);

    let ltc_configuration = ltc2944::Configuration {
        adc_mode: ltc2944::ADCMode::Scan,
        prescaler: ltc2944::Prescaler::Prescale4096,
        nalcc_configuration: ltc2944::NALCCConfiguration::Alert,
        shutdown: false,
    };
    let ltc = LTC2944::new(&mut i2c_con, Some(ltc_configuration), LTC_R_SENSE_MOHM).await;

    startup_sequence(&mut i2c_con).await;

    // Enable outputs. The 24V and 5V networks end up unused on this board, so we just close both
    // switches once at startup and leave them there (no dynamic fault monitoring).
    output_5v.set_high();
    output_24v.set_high();

    info!("5V status: {}", output_5v.get_output_level());
    info!("24V status: {}", output_24v.get_output_level());

    // TODO: update to the actual battery temperature sensor pin once the schematic is finalized.
    let mut one_wire_bus = OneWireBus::init(p.PB0);
    let mut battery_temp_sensor = DS18B20::new(None);
    if let Err(e) = battery_temp_sensor
        .ensure_config(&mut one_wire_bus, ds18b20::Config::default())
        .await
    {
        error!("Failed to configure battery temperature sensor: {}", e);
    }

    let mut can_configurator = can::CanConfigurator::new(p.FDCAN1, p.PA11, p.PA12, Irqs);
    can_configurator.set_bitrate(CAN_BITRATE);
    let can = can_configurator.start(can::OperatingMode::NormalOperationMode);

    static CAN_TX_BUF: StaticCell<can::TxBuf<16>> = StaticCell::new();
    static CAN_RX_BUF: StaticCell<can::RxBuf<1>> = StaticCell::new();
    let can = can.buffered(
        CAN_TX_BUF.init(can::TxBuf::<16>::new()),
        CAN_RX_BUF.init(can::RxBuf::<1>::new()),
    );

    spawner.spawn(unwrap!(battery_task(i2c_con, ltc, can.writer())));
    spawner.spawn(unwrap!(battery_data_task(can.writer())));
    spawner.spawn(unwrap!(temperature_task(one_wire_bus, battery_temp_sensor, can.writer())));

    // TODO: tasks still missing:
    // - EEPROM task: regularly save the voltage and charge
    // - LED task: blinks the LED every second, stop blinking in case of fault (determined by CAN
    // task via signal)

    // EEPROM Task should write every 10s or when asked by the CAN task: use embassy-future:select
    // and signals for that.
    //
    // For sharing i2c bus amongst devices, see:
    // - https://github.com/embassy-rs/embassy/blob/main/examples/rp/src/bin/shared_bus.rs
    // - https://docs.embassy.dev/embassy-embedded-hal/git/default/shared_bus/asynch/i2c/index.html

    loop {
        status_led.toggle();
        Timer::after_millis(1000).await;
    }
}
