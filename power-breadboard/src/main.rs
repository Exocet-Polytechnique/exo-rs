// TODO:
// 1. Read battery temperature every 500 ms, etc.
// 2. Process messages from IMD CAN
// 3. Handle regular network CAN + state machine
// 4. Control contactors during startup and shutdown and allow interrupting (event loop?)
// 5. Read data from fuel cell via UART
// 6. Configure MPPT at startupt (detect when it connects), then read data from it
// 7. Detect DMS at the very beginning of startup, ensure it is in a correct state after. Monitor
//    the alarm signal and read dms status/fault latch

#![no_std]
#![no_main]

pub mod tasks;
pub mod util;

use defmt::*;
use embassy_executor::Spawner;
use embassy_time::Timer;
use crate::tasks::{can::can_task, contactors::{ContactorOutputs, contactors_task}, fuel_cell::fuel_cell_task, imd::imd_task, mppt::mppt_task, safety::{SafetyGpios, safety_task}, temperature::temperature_task};

use {defmt_rtt as _, panic_probe as _};
use embassy_stm32::{Config, can, gpio::{Input, Level, Output, Pull, Speed}, i2c::{self, I2c}, time::Hertz, usart::{self, Uart}};
use embassy_stm32::{bind_interrupts, dma, peripherals};

pub mod dbc_gen {
    include!(concat!(env!("OUT_DIR"), "/dbc_gen.rs"));
}

bind_interrupts!(struct Irqs {
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
    DMA1_CHANNEL3 => dma::InterruptHandler<peripherals::DMA1_CH3>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
    DMA1_CHANNEL6 => dma::InterruptHandler<peripherals::DMA1_CH6>;
    FDCAN2_IT0 => can::IT0InterruptHandler<peripherals::FDCAN2>;
    FDCAN2_IT1 => can::IT1InterruptHandler<peripherals::FDCAN2>;
    FDCAN3_IT0 => can::IT0InterruptHandler<peripherals::FDCAN3>;
    FDCAN3_IT1 => can::IT1InterruptHandler<peripherals::FDCAN3>;
    UART4 => usart::InterruptHandler<peripherals::UART4>;
    UART5 => usart::InterruptHandler<peripherals::UART5>;
    I2C3_EV => i2c::EventInterruptHandler<peripherals::I2C3>;
    I2C3_ER => i2c::ErrorInterruptHandler<peripherals::I2C3>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(24_000_000), // 24-MHz oscillator
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV6,
            mul: PllMul::MUL85,
            divp: None,
            divq: Some(PllQDiv::DIV8), // 42.5 MHz for CAN
            divr: Some(PllRDiv::DIV2), // Main sys clock at 170 MHz
            // prediv: PllPreDiv::DIV3,
            // mul: PllMul::MUL20,
            // divp: Some(PllPDiv::DIV8), // P clock at 20 MHz
            // divq: Some(PllQDiv::DIV4), // CAN and peripheral clock at 40 MHz
            // divr: Some(PllRDiv::DIV2), // Main system clock at 80 MHz
        });
        config.rcc.mux.fdcansel = mux::Fdcansel::PLL1_Q;
        config.rcc.sys = Sysclk::PLL1_R;
    }
    let p = embassy_stm32::init(config);

    // 1. Initialize network CAN bus
    let mut can_config = can::CanConfigurator::new(p.FDCAN2, p.PB5, p.PB6, Irqs);
    can_config.set_bitrate(250_000);
    let can_bus = can_config.start(can::OperatingMode::NormalOperationMode);

    // 2. Initialize IMD CAN bus
    let mut imd_can_config = can::CanConfigurator::new(p.FDCAN3, p.PB3, p.PB4, Irqs);
    imd_can_config.set_bitrate(250_000);
    let imd_can_bus = imd_can_config.start(can::OperatingMode::NormalOperationMode);

    // 3. Initialize Fuel Cell UART
    let mut fc_uart_config = usart::Config::default();
    fc_uart_config.baudrate = 57_600;
    fc_uart_config.data_bits = usart::DataBits::DataBits8;
    fc_uart_config.stop_bits = usart::StopBits::STOP1;
    fc_uart_config.parity = usart::Parity::ParityNone;
    let fc_uart = Uart::new(p.UART4, p.PC11, p.PC10, p.DMA1_CH1, p.DMA1_CH2, Irqs, fc_uart_config).unwrap();

    // 4. Initialize IMD UART
    let mut mppt_uart_config = usart::Config::default();
    mppt_uart_config.baudrate = 19_200;
    mppt_uart_config.data_bits = usart::DataBits::DataBits8;
    mppt_uart_config.stop_bits = usart::StopBits::STOP1;
    mppt_uart_config.parity = usart::Parity::ParityNone;
    let mppt_uart = Uart::new(p.UART5, p.PD2, p.PC12, p.DMA1_CH3, p.DMA1_CH4, Irqs, mppt_uart_config).unwrap();

    // 5. Initialize temperature sensor i2c peripheral
    let i2c_config = i2c::Config::default();
    let temperature_i2c = I2c::new(p.I2C3, p.PC8, p.PC9, p.DMA1_CH5, p.DMA1_CH6, Irqs, i2c_config);


    // 6. Initialize status LED
    let mut status_led = Output::new(p.PA5, Level::Low, Speed::Low);

    // 7. Safety systems GPIOs
    let safety_gpios = SafetyGpios {
        // Inputs
        alarm_status: Input::new(p.PB7, Pull::None),
        dms_fault_latch: Input::new(p.PA0, Pull::None),
        dms_status: Input::new(p.PB0, Pull::None),

        // Outputs
        alarm_force: Output::new(p.PC2, Level::Low, Speed::Low),
        enable_dms: Output::new(p.PC3, Level::Low, Speed::Low),
        reset_dms: Output::new(p.PA1, Level::Low, Speed::Low),
    };

    // 8. Contactor GPIOs
    let contactor_ouputs = ContactorOutputs {
        bop: Output::new(p.PC6, Level::Low, Speed::Low),
        storage_battery: Output::new(p.PC5, Level::Low, Speed::Low),
        mppt_stage_1: Output::new(p.PA12, Level::Low, Speed::Low),
        mppt_stage_2: Output::new(p.PA11, Level::Low, Speed::Low),
        n_motor_stage_1: Output::new(p.PB12, Level::High, Speed::Low),
        n_motor_stage_2: Output::new(p.PB11, Level::High, Speed::Low),
        n_braking_resistor: Output::new(p.PB2, Level::High, Speed::Low),
        n_auxiliary_battery: Output::new(p.PB1, Level::High, Speed::Low),
    };

    // Tasks
    spawner.spawn(unwrap!(can_task(can_bus)));
    spawner.spawn(unwrap!(safety_task(safety_gpios)));
    spawner.spawn(unwrap!(imd_task(imd_can_bus)));
    spawner.spawn(unwrap!(fuel_cell_task(fc_uart)));
    spawner.spawn(unwrap!(mppt_task(mppt_uart)));
    // spawner.spawn(unwrap!(temperature_task(temperature_i2c)));
    spawner.spawn(unwrap!(contactors_task(contactor_ouputs)));

    loop {
        status_led.toggle();
        Timer::after_millis(1000).await;
    }
}
