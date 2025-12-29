#![no_std]
#![no_main]

#[cfg(not(feature = "use_semihosting"))]
use panic_halt as _;
#[cfg(feature = "use_semihosting")]
use panic_semihosting as _;

use arduino_mkr1000 as bsp;
use bsp::entry;
use bsp::hal;

use hal::clock::GenericClockController;
use hal::gpio::D;
use hal::pac::{CorePeripherals, Peripherals};
use hal::prelude::*; // C'est le bon !

// --- IMPORTS CRUCIAUX ---
// On ajoute 'Pads' ici
use hal::gpio::Pins;
use hal::sercom::uart::{BaudMode, Config, Oversampling, Pads};
use hal::sercom::Sercom4;

mod modem;
mod nb_modem;
use modem::Modem;
use nb_modem::NbModem;

use core::cell::RefCell;
use cortex_m::interrupt::Mutex;

static MILLIS: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0));

#[cortex_m_rt::exception]
fn SysTick() {
    cortex_m::interrupt::free(|cs| {
        *MILLIS.borrow(cs).borrow_mut() += 1;
    });
}

#[entry]
fn main() -> ! {
    let mut peripherals = Peripherals::take().unwrap();
    let mut core = CorePeripherals::take().unwrap();

    let mut clocks = GenericClockController::with_internal_32kosc(
        peripherals.GCLK,
        &mut peripherals.PM,
        &mut peripherals.SYSCTRL,
        &mut peripherals.NVMCTRL,
    );

    let reload_value = 48_000_000 / 1000;
    core.SYST.set_reload(reload_value);
    core.SYST.clear_current();
    core.SYST.enable_counter();
    core.SYST.enable_interrupt();

    let now_ms = || cortex_m::interrupt::free(|cs| *MILLIS.borrow(cs).borrow());

    // 1. PINS
    let mut pins = Pins::new(peripherals.PORT);

    // 2. CONFIGURATION DES PINS UART
    // RX = PA13, TX = PA12 en mode Alternate D
    let rx = pins.pa13.into_alternate::<D>();
    let tx = pins.pa12.into_alternate::<D>();

    // --- CORRECTION MAJEURE ICI ---
    // On ne fait pas un tuple (rx, tx).
    // On crée l'objet Pads spécifique au Sercom4.
    let uart_pads = Pads::<Sercom4>::default().rx(rx).tx(tx);

    // 3. Horloge
    let gclk0 = clocks.gclk0();
    let sercom4_clock = clocks.sercom4_core(&gclk0).unwrap();

    // 4. Config UART
    let config = Config::new(
        &mut peripherals.PM,
        peripherals.SERCOM4,
        uart_pads, // On passe l'objet Pads ici
        sercom4_clock.freq(),
    );

    // --- CORRECTION FREQUENCE ---
    // Utilisation de Hertz(...) pour être sûr que ça marche
    let config = config.baud(115200u32.Hz(), BaudMode::Fractional(Oversampling::Bits16));

    let serial_sara = config.enable();

    // 5. PINS DE CONTRÔLE
    // Note: into_push_pull_output() a parfois besoin de l'argument (&mut pins.port)
    // Si ça plante, rajoute l'argument. Sinon laisse vide.
    let sara_pwr = pins.pb08.into_push_pull_output();
    let sara_reset = pins.pa27.into_push_pull_output();
    let mut led = pins.pa20.into_push_pull_output();

    // 6. INITIALISATION MODEM
    let modem_driver = Modem::new(serial_sara, sara_reset, sara_pwr, None::<DummyPin>);
    let mut my_nb_modem = NbModem::new(modem_driver);

    let mut success = false;

    if my_nb_modem.begin(&now_ms).is_ok() {
        if let Ok(imei) = my_nb_modem.get_imei(&now_ms) {
            if imei.len() > 10 {
                success = true;
            }
        }
    }

    loop {
        if success {
            led.set_high().unwrap();
        } else {
            led.set_high().unwrap();
            delay_busy(200_000);
            led.set_low().unwrap();
            delay_busy(200_000);
        }
    }
}

fn delay_busy(count: u32) {
    for _ in 0..(count * 50) {
        cortex_m::asm::nop();
    }
}
// --- Structure Dummy pour satisfaire le compilateur ---
struct DummyPin;

// On dit à Rust que DummyPin se comporte comme une InputPin
impl embedded_hal::digital::v2::InputPin for DummyPin {
    type Error = ();
    fn is_high(&self) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn is_low(&self) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
