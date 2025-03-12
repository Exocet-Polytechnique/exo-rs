#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{adc::{Adc, SampleTime}, gpio::{Input, Level, Output, Pull, Speed}, peripherals::{ADC2, PA10, PA4, PA8, PA9, PB13, PB14, PB5, PC13}};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

static mut ERROR_FLAG: bool = false;

#[derive(Debug)]
enum State {
    Idle,
    VerifyAllClose,
    VerifyActOpen,
    VerifyActClose,
    Active,
    Error
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {

    let p = embassy_stm32::init(Default::default());
    
    spawner.spawn(state_machine(spawner, p.PC13, p.PB5, p.PA10, p.PB13, p.PB14, 
        p.PA8, p.PA9)).unwrap(); // Tâche: Machine à états

    spawner.spawn(monitor_adc(spawner, p.ADC2, p.PA4)).unwrap(); // Tâche: Lecture constante du senseur
}

#[embassy_executor::task]
async fn monitor_adc(_spawner: Spawner, _adc_2: ADC2, _pin_a4: PA4) {
    // let mut pressure_sensor = Adc::new(adc_2);
    // pressure_sensor.set_sample_time(SampleTime::CYCLES24_5);
    // let mut pin = pin_a4;
    // loop {
    //     let measured = pressure_sensor.read(&mut pin).await.unwrap();
    //     info!("measured: {}", measured);
    //     Timer::after_millis(500).await;
    //     }
    // TODO: Implementer la lecture constante du senseur.
    }

#[embassy_executor::task]
async fn state_machine(_spawner: Spawner, pin_c13:PC13, pin_b5:PB5, pin_a10: PA10, pin_b13: PB13, pin_b14: PB14, 
    pin_a8: PA8, pin_a9: PA9) {

    let button = Input::new(pin_c13, Pull::Down);
    let act1_in = Input::new(pin_b5, Pull::Down);
    let act2_in = Input::new(pin_a10, Pull::Down);
    let mut act1_out = Output::new(pin_b13, Level::High, Speed::Low);
    let mut act2_out = Output::new(pin_b14, Level::High, Speed::Low);
    let mut led = Output::new(pin_a8, Level::Low, Speed::Low);
    let mut led_err = Output::new(pin_a9, Level::Low, Speed::Low);
    
    let mut state = State::Idle;
    let mut act_counter: i8 = 0; 
    loop { 

        if unsafe { ERROR_FLAG } {
            state = State::Error;
        }

        match state{
            State::Idle=>{

                if button.is_high(){
                    state = State::VerifyAllClose;
                }

                led.set_low();
                Timer::after_millis(10).await;
            }

            State::Active=>{
                info!("Mode actif");
                led.set_high();
                Timer::after_millis(10).await;
            }

            State::VerifyAllClose=>{

                info!("Verification des actuateurs dans 3s");
                Timer::after_millis(3000).await;

                if act1_in.is_low() || act2_in.is_low() {
                    state = State::Error;
                } else {
                    state = State::VerifyActOpen;
                }
            }

            State::VerifyActOpen=>{

                if act_counter == 0 {
                    act1_out.set_high();
                    info!("Ouverture de l'actuateur 1");
                    Timer::after_millis(1000).await;
                    if act1_in.is_low(){
                        state = State::Error;
                    } else {
                        state = State::VerifyActClose;
                        info!("Actuateur 1 valide");
                    }
                }

                else if act_counter == 1 {
                    act2_out.set_high();
                    info!("Ouverture de l'actuateur 2");
                    Timer::after_millis(1000).await;
                    if act2_in.is_low(){
                        state = State::Error;
                    } else {
                        state = State::VerifyActClose;
                        info!("Actuateur 2 valide");
                    }
                }

                else {
                    state = State::Error;
                }
            }

            State::VerifyActClose=>{
                if act_counter == 0 {
                    act1_out.set_low();
                    Timer::after_millis(1000).await;
                    if act1_in.is_high(){
                        state = State::Error;
                    } else {
                        act_counter += 1;
                        state = State::VerifyActOpen;
                    }
                } 

                else if act_counter == 1 {
                    act2_out.set_low();
                    Timer::after_millis(1000).await;
                    if act2_in.is_high(){
                        state = State::Error;
                    } else {
                        state = State::Active;
                    }
                } 

                else {
                    state = State::Error;
                }
            }

            State::Error=>{
                act1_out.set_low();
                act2_out.set_low();
                led_err.set_high();
                Timer::after_millis(10).await;
            }
        }
        Timer::after_millis(10).await;
     }
}