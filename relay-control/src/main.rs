#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::{Spawner};
use embassy_stm32::{
    gpio::{Input, Level, Output, Pull, Speed}
};
use embassy_time::Timer;

use {defmt_rtt as _, panic_probe as _};

#[derive(Debug)]
enum State {
    Idle,
    FirstState,
    SecondState,
    ThirdState,
    FourthState,
    WaitingForSecondState,
    WaitingForThirdState,
    ToggleState,
}

#[embassy_executor::main]
async fn main(_spawner : Spawner) {

    let p = embassy_stm32::init(Default::default());

    let mut user_del = Output::new(p.PA5, Level::Low, Speed::Low);

    let user_bnt = Input::new(p.PC13, Pull::Down);

    let mut relay1 = Output::new(p.PC5, Level::Low, Speed::Low);
    let mut relay2 = Output::new(p.PC4, Level::Low, Speed::Low);
    let mut relay3 = Output::new(p.PA10, Level::Low, Speed::Low);
    let mut relay4 = Output::new(p.PB3, Level::Low, Speed::Low);
    let mut relay5 = Output::new(p.PB5, Level::High, Speed::Low);
    let mut relay6 = Output::new(p.PB4, Level::High, Speed::Low);

    let mut state = State::Idle;

    loop {

        match state {
            State::Idle => if user_bnt.is_high() {
                state = State::FirstState;
                println!("Going to FirstState");
            }
            State::FirstState => {
                user_del.set_high();
                relay1.set_high();
                Timer::after_secs(30).await;
                user_del.set_low();
                state = State::WaitingForSecondState;
            }
            State::WaitingForSecondState => if user_bnt.is_high() {
                state = State::SecondState;
                println!("Going to SecondState");
            }
            State::SecondState => {
                user_del.set_high();
                relay2.set_high();
                Timer::after_secs(30).await;
                relay3.set_high();
                Timer::after_secs(2).await;
                user_del.set_low();
                relay2.set_low();
                state = State::WaitingForThirdState;
            }
            State::WaitingForThirdState => if user_bnt.is_high() {
                state = State::ThirdState;
                println!("Going to ThirdState");
            }
            State::ThirdState => {
                relay4.set_high();
                user_del.set_high();
                Timer::after_secs(2).await;
                relay5.set_low();
                Timer::after_secs(30).await;
                relay6.set_low();
                Timer::after_secs(2).await;
                user_del.set_low();
                relay4.set_low();
                state = State::ToggleState;
            }
            State::ToggleState => {
                Timer::after_millis(500).await;
                user_del.toggle();
                if user_bnt.is_high() {
                    state = State::FourthState;
                    println!("Going to FourthState");
                }
            }
            State::FourthState => {
                relay4.set_high();
                user_del.set_high();
                Timer::after_secs(2).await;
                relay6.set_high();
                Timer::after_secs(2).await;
                relay5.set_high();
                Timer::after_secs(2).await;
                relay4.set_low();
                relay2.set_high();
                Timer::after_secs(2).await;
                relay3.set_low();
                Timer::after_secs(2).await;
                relay2.set_low();
                Timer::after_secs(2).await;
                relay1.set_low();
                println!("Going to Idle");
                state = State::Idle;
            }
        }
    }
}