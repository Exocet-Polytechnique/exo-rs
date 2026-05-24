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
use embassy_stm32::{Config, Peri, adc::{Adc, AdcChannel, AnyAdcChannel, SampleTime}, gpio::{Level, Output, Speed}, peripherals::{ADC1, ADC2, DMA1_CH1}};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, priority_channel::PriorityChannel};
use embassy_time::Timer;
use heapless::binary_heap::Min;
use {defmt_rtt as _, panic_probe as _};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, peripherals};

bind_interrupts!(struct Irqs {
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
});

const ADC_MAX: f32 = 3640.0;

enum SensorData {
    Manometer1(f32),
    Manometer2(f32),
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

static MESSAGE_CHANNEL: PriorityChannel<ThreadModeRawMutex, SensorMessage, Min, 5> = PriorityChannel::new();

#[embassy_executor::task]
async fn can_task() {
    info!("CAN task started...");

    loop {
        let message = MESSAGE_CHANNEL.receive().await;

        match message.data {
            SensorData::Manometer1(pressure) => {
                info!("Receive pressure value from manometer 1: {}", pressure);
            }
            SensorData::Manometer2(pressure) => {
                info!("Receive pressure value from manometer 2: {}", pressure);
            }
        }
    }
}

#[embassy_executor::task]
async fn pressure_task(mut adc1: Adc<'static, ADC1>, mut adc2: Adc<'static, ADC2>, mut pa0: AnyAdcChannel<'static, ADC1>, mut pa1: AnyAdcChannel<'static, ADC2>) {
    info!("Starting pressure task");

    loop {
        // TODO: convert to bars
        let adc1_v = adc1.blocking_read(&mut pa0, SampleTime::CYCLES247_5) as f32 / ADC_MAX;
        MESSAGE_CHANNEL.send(SensorMessage{
            priority: 0,
            data: SensorData::Manometer1(adc1_v)
        }).await;
        let adc2_v = adc2.blocking_read(&mut pa1, SampleTime::CYCLES247_5) as f32 / ADC_MAX;
        MESSAGE_CHANNEL.send(SensorMessage{
            priority: 1,
            data: SensorData::Manometer2(adc2_v)
        }).await;

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

    // TODO: temperature sensor driver

    let mut status_led = Output::new(p.PC0, Level::Low, Speed::Low);

    let adc1 = Adc::new(p.ADC1, Default::default());
    let adc2 = Adc::new(p.ADC2, Default::default());

    let pa0 = p.PA0.degrade_adc();
    let pa1 = p.PA1.degrade_adc();

    spawner.spawn(unwrap!(can_task()));
    spawner.spawn(unwrap!(pressure_task(adc1, adc2, pa0, pa1)));

    loop {
        status_led.toggle();
        Timer::after_millis(1000).await;
    }
}
