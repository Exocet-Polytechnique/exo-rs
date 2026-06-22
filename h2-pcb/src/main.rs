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
use embassy_futures::select::{Either, select};
use embassy_stm32::{Config, Peri, adc::{Adc, AdcChannel, AnyAdcChannel, SampleTime}, can, gpio::{Input, Level, Output, Speed}, i2c::{self, I2c, Config as I2cConfig}, peripherals::{ADC1, ADC2, DMA1_CH1, FDCAN1, PB4, PB5, PB6, PB9, PC1, PC2, PC3, PC6, PC7}};
use embassy_sync::{blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex}, channel::Channel, priority_channel::PriorityChannel, signal::Signal, watch::Watch};
use embassy_time::Timer;
use embedded_hal::can::{Id, StandardId};
use heapless::binary_heap::Min;
use {defmt_rtt as _, panic_probe as _};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, peripherals};
use exo_drivers::{
    ds2484::{DS2484},
    ds18b20::{self, DS18B20}
};

bind_interrupts!(struct CanIrqs {
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

bind_interrupts!(struct I2cIrqs {
    I2C2_EV => i2c::EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => i2c::ErrorInterruptHandler<peripherals::I2C2>;
    DMA1_CHANNEL6 => dma::InterruptHandler<peripherals::DMA1_CH6>;
    DMA1_CHANNEL7 => dma::InterruptHandler<peripherals::DMA1_CH7>;
});

const ADC_MAX: f32 = 3640.0;

enum SensorData {
    Manometers(f32, f32),
    Actuators(u8, u8, u8),
    Thermometers(f32, f32, f32),
}

struct SensorMessage {
    priority: u32,
    data: SensorData,
}

impl PartialEq for SensorMessage {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for SensorMessage {}

impl PartialOrd for SensorMessage {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.priority.partial_cmp(&other.priority)
    }
}

impl Ord for SensorMessage {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

struct ActuatorMessage {
    id: u8,
    status: bool,
}

static MESSAGE_CHANNEL: PriorityChannel<ThreadModeRawMutex, SensorMessage, Min, 5> = PriorityChannel::new();
static ACTUATOR_CHANNEL: Channel<ThreadModeRawMutex, ActuatorMessage, 5> = Channel::new();
static MANO_WATCH: Watch<CriticalSectionRawMutex, bool, 2> = Watch::new();
static THERMO_WATCH: Watch<CriticalSectionRawMutex, bool, 3> = Watch::new();

const ACTUATOR_COMMAND: u8 = 0x00;
const ACTUATOR_READ_COMMAND: u8 = 0x01;
const MANO_READ_COMMAND: u8 = 0x02;
const THERMO_READ_COMMAND: u8 = 0x03;

#[embassy_executor::task]
async fn can_task(mut can_bus: can::Can<'static>) {
    info!("CAN task started...");

    loop {
        match select(MESSAGE_CHANNEL.receive(), can_bus.read()).await {
            Either::First(message) => {
                match message.data {
                    SensorData::Manometers(v1, v2) => {
                        let v1_int: u16 = (v1 * 1000.0) as u16;
                        let v2_int: u16 = (v2 * 1000.0) as u16;

                        let frame = can::frame::Frame::new_standard(0x01, &[MANO_READ_COMMAND, ((v1_int >> 8) & 0xFF) as u8, (v1_int & 0xFF) as u8, ((v2_int >> 8) & 0xFF) as u8, (v2_int & 0xFF) as u8]).unwrap();
                        can_bus.write(&frame).await;
                    }
                    SensorData::Actuators(a1, a2, a3) => {
                        let frame = can::frame::Frame::new_standard(0x01, &[ACTUATOR_READ_COMMAND, a1, a2, a3]).unwrap();
                        can_bus.write(&frame).await;
                    }
                    SensorData::Thermometers(t1,t2 ,t3 ) => {
                        let t1_int: u16 = (t1 * 1000.0) as u16;
                        let t2_int: u16 = (t2 * 1000.0) as u16;
                        let t3_int: u16 = (t3 * 1000.0) as u16;

                        let frame = can::frame::Frame::new_standard(0x01, &[THERMO_READ_COMMAND, ((t1_int >> 8) & 0xFF) as u8, (t1_int & 0xFF) as u8, ((t2_int >> 8) & 0xFF) as u8, (t2_int & 0xFF) as u8, ((t3_int >> 8) & 0xFF) as u8, (t3_int & 0xFF) as u8]).unwrap();
                        can_bus.write(&frame).await;
                    }
                }
            }
            Either::Second(recv_data) => {
                match recv_data {
                    Ok(envelope) => {
                        match envelope.frame.data()[0] {
                            ACTUATOR_COMMAND => {
                                ACTUATOR_CHANNEL.send(ActuatorMessage { id: envelope.frame.data()[1], status: (envelope.frame.data()[2] == 1) }).await;
                            }
                            ACTUATOR_READ_COMMAND => {
                                ACTUATOR_CHANNEL.send(ActuatorMessage { id: 4, status: false }).await;
                            }
                            MANO_READ_COMMAND => {
                                MANO_WATCH.sender().send(envelope.frame.data()[1] == 1);
                            }
                            THERMO_READ_COMMAND => {
                                THERMO_WATCH.sender().send(envelope.frame.data()[1] == 1);
                            }
                            _ => {
                                info!("Unknown message")
                            }
                        }
                    }
                    Err(_) => {
                        error!("CAN error");
                    }
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn pressure_task(mut adc1: Adc<'static, ADC1>, mut adc2: Adc<'static, ADC2>, mut pa0: AnyAdcChannel<'static, ADC1>, mut pa1: AnyAdcChannel<'static, ADC2>) {
    info!("Starting pressure task");

    let mut recv = MANO_WATCH.receiver().unwrap();

    loop {
        // TODO: convert to bars
        if recv.get().await {
            let adc1_v = (adc1.blocking_read(&mut pa0, SampleTime::CYCLES247_5) as f32 / ADC_MAX) * 3.95;
            let adc2_v = (adc2.blocking_read(&mut pa1, SampleTime::CYCLES247_5) as f32 / ADC_MAX) * 371.35;
            MESSAGE_CHANNEL.send(SensorMessage{
                priority: 0,
                data: SensorData::Manometers(adc1_v, adc2_v)
            }).await;
        }

        Timer::after_millis(200).await;

        // let mut adc_reading: [u16; 1] = [0; 1];
        // let adc_value = adc1.blocking_read(&mut pa0, SampleTime::CYCLES247_5);
        // info!("adc_value: {}", adc_value);
        //
        //
        // adc1.read(dma.reborrow(), Irqs, [
        //     (&mut pa0, SampleTime::CYCLES247_5)
        // ].into_iter(), &mut adc_reading).await;
        //
        // // TODO: convert ADC reading to bars
        // let manometer1_pressure = (adc_reading[0] as f32) / (u16::MAX as f32);
        // MESSAGE_CHANNEL.send(SensorMessage{
        //     priority: 0,
        //     data: SensorData::Manometer1(manometer1_pressure)
        // }).await;
        //
        // adc2.read(dma.reborrow(), Irqs, [
        //     (&mut pa1, SampleTime::CYCLES247_5)
        // ].into_iter(), &mut adc_reading).await;
        //
        // // TODO: convert ADC reading to bars
        // let manometer2_pressure = (adc_reading[0] as f32) / (u16::MAX as f32);
        // MESSAGE_CHANNEL.send(SensorMessage{
        //     priority: 0,
        //     data: SensorData::Manometer2(manometer2_pressure)
        // }).await;
        //
        // Timer::after_millis(100).await; // Check pressure every 100 ms
    }
}

#[embassy_executor::task]
async fn actuator_task(act1: Peri<'static, PC1>, act2: Peri<'static, PC2>, act3: Peri<'static, PC3>, no1: Peri<'static, PB4>, nc1: Peri<'static, PB5>, no2: Peri<'static, PB6>, nc2: Peri<'static, PB9>, no3: Peri<'static, PC6>, nc3: Peri<'static, PC7>) {
    info!("Starting actuator task");

    let read_no1 = Input::new(no1, embassy_stm32::gpio::Pull::None);
    let read_nc1 = Input::new(nc1, embassy_stm32::gpio::Pull::None);
    let read_no2 = Input::new(no2, embassy_stm32::gpio::Pull::None);
    let read_nc2 = Input::new(nc2, embassy_stm32::gpio::Pull::None);
    let read_no3 = Input::new(no3, embassy_stm32::gpio::Pull::None);
    let read_nc3 = Input::new(nc3, embassy_stm32::gpio::Pull::None);

    let mut act1_out = Output::new(act1, Level::Low, Speed::Low);
    let mut act2_out = Output::new(act2, Level::Low, Speed::Low);
    let mut act3_out = Output::new(act3, Level::Low, Speed::Low);

    loop {
        let data = ACTUATOR_CHANNEL.receive().await;

        if data.id == 4 {
            MESSAGE_CHANNEL.send(SensorMessage {
                priority: 1,
                data: SensorData::Actuators(
                    if read_nc1.is_high() {
                        0
                    } else if read_no1.is_high() {
                        1
                    } else {
                        2
                    },
                    if read_nc2.is_high() {
                        0
                    } else if read_no2.is_high() {
                        1
                    } else {
                        2
                    },
                    if read_nc3.is_high() {
                        0
                    } else if read_no3.is_high() {
                        1
                    } else {
                        2
                    },
                )
            }).await;
        } else {
            match data.id {
                1 => {
                    // act1_out.set_level(if data.status { Level::High } else { Level::Low });
                    act1_out.set_high();
                    info!("Act 1");
                    MESSAGE_CHANNEL.send(SensorMessage {
                        priority: 2,
                        data: SensorData::Actuators(3, 4, 4),
                    }).await;

                }
                2 => {
                    act2_out.set_high();
                    info!("Act 2");
                    // act2_out.set_level(if data.status { Level::High } else { Level::Low });
                    MESSAGE_CHANNEL.send(SensorMessage {
                        priority: 2,
                        data: SensorData::Actuators(4, 3, 4),
                    }).await;
                }
                3 => {
                    act3_out.set_high();
                    info!("Act 3");
                    // act3_out.set_level(if data.status { Level::High } else { Level::Low });
                    MESSAGE_CHANNEL.send(SensorMessage {
                        priority: 2,
                        data: SensorData::Actuators(4, 4, 3),
                    }).await;
                }
                _ => {}
            }
        }
    }
}

#[embassy_executor::task]
async fn temperature_task(mut bus: DS2484, mut sensors: [DS18B20; 3]) {
    info!("Starting temperature task");

    let mut recv = THERMO_WATCH.receiver().unwrap();
    let mut temperatures: [f32; 3] = [0.0; 3];

    loop {
        if recv.get().await {

            for i in 0..3 {
                match sensors[i].read_temperature(&mut bus).await {
                    Err(e) => info!("Failed to read temperature: {}", e),
                    Ok(t) => {
                        temperatures[i] = t;
                    }
                }
            }

            MESSAGE_CHANNEL.send(SensorMessage{
                priority: 1,
                data: SensorData::Thermometers(temperatures[0], temperatures[1], temperatures[2]),
            }).await;

        }
        Timer::after_millis(200).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000), // 8-MHz oscillator
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV2,
            mul: PllMul::MUL85,
            divp: None,
            divq: Some(PllQDiv::DIV8), // 42.5 MHz for CAN
            divr: Some(PllRDiv::DIV2), // Main system clock at 170 MHz
        });
        config.rcc.mux.adc12sel = mux::Adcsel::SYS;
        config.rcc.mux.fdcansel = mux::Fdcansel::PLL1_Q;
        config.rcc.sys = Sysclk::PLL1_R;
    }
    let p = embassy_stm32::init(config);

    let mut can_bus = can::CanConfigurator::new(p.FDCAN1, p.PA11, p.PA12, CanIrqs);
    can_bus.properties().set_extended_filter(can::filter::ExtendedFilterSlot::_0, can::filter::ExtendedFilter::accept_all_into_fifo1());
    can_bus.set_bitrate(250_000);

    let can_bus_final = can_bus.start(can::OperatingMode::NormalOperationMode);

    // TODO: temperature sensor driver

    let mut status_led = Output::new(p.PC0, Level::Low, Speed::Low);

    let adc1 = Adc::new(p.ADC1, Default::default());
    let adc2 = Adc::new(p.ADC2, Default::default());

    let pa0 = p.PA0.degrade_adc();
    let pa1 = p.PA1.degrade_adc();

    let act1 = p.PC1;
    let act2 = p.PC2;
    let act3 = p.PC3;
    let no1 = p.PB4;
    let no2 = p.PB6;
    let no3 = p.PC6;
    let nc1 = p.PB5;
    let nc2 = p.PB9;
    let nc3 = p.PC7;

    let i2c = I2c::new(
        p.I2C2,
        p.PC4,                      // SCL
        p.PA8,                      // SDA
        p.DMA1_CH6,              // TX DMA
        p.DMA1_CH7,              // RX DMA
        I2cIrqs,                   // IRQ — moved after DMA
        I2cConfig::default(),
    );

    let mut bus = DS2484::new(i2c);

    // TODO: Change Sensors' address with real one

    const SENSOR1: u64 = 0x28FF123456789ABC;
    const SENSOR2: u64 = 0x28FF9876543210DE;
    const SENSOR3: u64 = 0x28FF111122223333;

    let mut sensors = [
        DS18B20::new(Some(SENSOR1)),
        DS18B20::new(Some(SENSOR2)),
        DS18B20::new(Some(SENSOR3)),
    ];

    for i in 0..3 {
        match sensors[i].ensure_config(
            &mut bus,
            ds18b20::Config::new(ds18b20::Resolution::TwelveBits),
        ).await {
            Ok(())  => defmt::info!("config set"),
            Err(e)  => defmt::panic!("ensure_config failed: {:?}", e),
        }
    }

    spawner.spawn(unwrap!(can_task(can_bus_final)));
    spawner.spawn(unwrap!(pressure_task(adc1, adc2, pa0, pa1)));
    spawner.spawn(unwrap!(actuator_task(act1, act2, act3, no1, nc1, no2, nc2, no3, nc3)));
    spawner.spawn(unwrap!(temperature_task(bus, sensors)));

    loop {
        status_led.toggle();
        Timer::after_millis(1000).await;
    }
}
