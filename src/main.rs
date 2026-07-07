#![no_std]
#![no_main]

pub mod tasks;

use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    can::{self, filter::{StandardFilter, StandardFilterSlot, FilterType, Action}},
    peripherals::FDCAN1,
};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

use crate::tasks::{can::can_reader, cockpit::cockpit};

pub mod dbc_gen {
    include!(concat!(env!("OUT_DIR"), "/dbc_gen.rs"));
}

bind_interrupts!(struct Irqs {
    FDCAN1_IT0 => can::IT0InterruptHandler<FDCAN1>;
    FDCAN1_IT1 => can::IT1InterruptHandler<FDCAN1>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.mux.fdcansel = mux::Fdcansel::PCLK1;
    }
    let p = embassy_stm32::init(config);

    let mut configurator = can::CanConfigurator::new(p.FDCAN1, p.PA11, p.PA12, Irqs);
    configurator.set_bitrate(250_000);

    // Configure CAN filter to accept only DriverInterfaceHAT's frames (HP_PCB05_E=271,
    // LP_PCB05_E=1295, LP_PCB05_P=1311) and reject all others. Only LP_PCB05_P is acted on
    // today; the error/warning frames are admitted for future use.
    // Mask 0x3EF covers every bit the three IDs share; the remaining two bits are
    // exactly what distinguishes them from each other, so no other message ID matches.
    configurator.properties().set_standard_filter(
        StandardFilterSlot::_0,
        StandardFilter {
            filter: FilterType::BitMask { filter: 271_u16, mask: 0x3EF },
            action: Action::StoreInFifo0,
        },
    );
    // Slot 1: catch-all reject — frames not matched by slot 0 are discarded
    configurator.properties().set_standard_filter(
        StandardFilterSlot::_1,
        StandardFilter {
            filter: FilterType::BitMask { filter: 0_u16, mask: 0_u16 },
            action: Action::Reject,
        },
    );
    let can: can::Can<'static> = configurator.start(can::OperatingMode::NormalOperationMode);
    let (tx, rx, _props) = can.split();

    spawner.spawn(can_reader(rx)).unwrap();
    spawner
        .spawn(cockpit(spawner, p.PA8, p.PA6, p.PA7, p.PB13, p.PB11, tx))
        .unwrap();

    loop {
        Timer::after_millis(1000).await;
    }
}
