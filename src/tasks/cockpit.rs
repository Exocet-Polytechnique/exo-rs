use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    can,
    gpio::{Input, Level, Output, Pull, Speed},
    peripherals::{PA6, PA7, PA8, PB11, PB13},
};
use embassy_futures::select::{select, Either};
use embassy_time::Timer;

use crate::dbc_gen;
use crate::tasks::can::{CAN_CHANNEL, CanEvent, send_command, send_state};

static mut ERROR_FLAG: bool = false;

#[derive(Debug)]
enum State {
    IDLE,
    STARTING,
    RUNNING,
    FAULT,
}

/// Normal shutdown initiated by the cockpit button.
/// Sends a Shutdown Command to DriverInterfaceHAT, waits for its CurrentState to announce Idle,
/// then confirms back with our own CurrentState.
async fn normal_shutdown(
    tx: &mut can::CanTx<'static>,
    led_g: &mut Output<'_>,
    led_r: &mut Output<'_>,
    led_y: &mut Output<'_>,
) {
    send_command(
        tx,
        dbc_gen::LpPcb01PTargetModule::DriverInterfaceHat,
        dbc_gen::LpPcb01PCommand::Shutdown,
    )
    .await;
    info!("Shutdown Command sent to DriverInterfaceHAT");
    loop {
        match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
            Either::First(CanEvent::DashboardState(dbc_gen::LpPcb05PCurrentState::Idle)) => {
                info!("Dashboard announced Idle, confirming");
                send_state(tx, dbc_gen::LpPcb01PCurrentState::Idle).await;
                break;
            }
            Either::First(_) => {}
            Either::Second(_) => {
                led_r.toggle();
                led_g.set_low();
                led_y.set_low();
            }
        }
    }
    led_r.set_low();
}

/// Force shutdown initiated by DriverInterface.
/// Immediately stops the current operation and resets LEDs for IDLE.
fn force_shutdown(led_g: &mut Output<'_>, led_r: &mut Output<'_>, led_y: &mut Output<'_>) {
    led_g.set_low();
    led_r.set_low();
    led_y.set_low();
    info!("Force shutdown by DriverInterface, returning to IDLE");
}

#[embassy_executor::task]
pub async fn cockpit(
    _spawner: Spawner,
    pin_a8: PA8,
    pin_a6: PA6,
    pin_a7: PA7,
    pin_b13: PB13,
    pin_b11: PB11,
    mut tx: can::CanTx<'static>,
) {
    info!("Démarrage du système...");

    let mut led_g = Output::new(pin_a8, Level::Low, Speed::Low);
    let mut led_y = Output::new(pin_a6, Level::Low, Speed::Low);
    let mut led_r = Output::new(pin_a7, Level::Low, Speed::Low);

    let button_g = Input::new(pin_b13, Pull::Down);
    let button_r = Input::new(pin_b11, Pull::Down);

    let mut state = State::IDLE;
    loop {
        if unsafe { ERROR_FLAG } {
            state = State::FAULT;
        }

        match state {
            State::IDLE => {
                led_r.set_high();
                led_g.set_low();
                led_y.set_low();

                // Drain CAN_CHANNEL even while idle so an out-of-band frame never fills it and
                // stalls can_reader.
                match select(CAN_CHANNEL.receive(), Timer::after_millis(50)).await {
                    Either::First(CanEvent::CriticalError) => {
                        unsafe { ERROR_FLAG = true; }
                    }
                    Either::First(_) => {}
                    Either::Second(_) => {
                        if button_g.is_high() {
                            send_command(
                                &mut tx,
                                dbc_gen::LpPcb01PTargetModule::DriverInterfaceHat,
                                dbc_gen::LpPcb01PCommand::Start,
                            )
                            .await;
                            info!("Start Command sent to DriverInterfaceHAT");
                            state = State::STARTING;
                        }
                    }
                }
            }

            State::STARTING => {
                match select(CAN_CHANNEL.receive(), Timer::after_millis(300)).await {
                    Either::First(CanEvent::DashboardState(dbc_gen::LpPcb05PCurrentState::Started)) => {
                        info!("Dashboard announced Started, confirming and entering RUNNING");
                        send_state(&mut tx, dbc_gen::LpPcb01PCurrentState::Running).await;
                        led_g.set_low();
                        state = State::RUNNING;
                    }
                    Either::First(CanEvent::ForceShutdown) => {
                        force_shutdown(&mut led_g, &mut led_r, &mut led_y);
                        state = State::IDLE;
                    }
                    Either::First(CanEvent::CriticalError) => {
                        unsafe { ERROR_FLAG = true; }
                    }
                    Either::First(_) => {}
                    Either::Second(_) => {
                        // Still waiting: blink green
                        led_g.toggle();
                        led_r.set_low();
                        led_y.set_low();
                    }
                }
            }

            State::RUNNING => {
                led_g.set_high();
                led_r.set_low();
                led_y.set_low();

                match select(CAN_CHANNEL.receive(), Timer::after_millis(50)).await {
                    Either::First(CanEvent::ForceShutdown) => {
                        force_shutdown(&mut led_g, &mut led_r, &mut led_y);
                        state = State::IDLE;
                    }
                    Either::First(CanEvent::CriticalError) => {
                        unsafe { ERROR_FLAG = true; }
                    }
                    Either::First(_) => {}
                    Either::Second(_) => {
                        if button_r.is_high() {
                            normal_shutdown(&mut tx, &mut led_g, &mut led_r, &mut led_y).await;
                            state = State::IDLE;
                        }
                    }
                }
            }

            State::FAULT => {
                led_g.set_low();
                led_y.set_low();
                led_r.toggle();
                info!("Erreur détectée, clignotement de la LED rouge");
                // Keep draining CAN_CHANNEL so a live dashboard doesn't back up while faulted.
                let _ = select(CAN_CHANNEL.receive(), Timer::after_millis(10)).await;
            }
        }
    }
}
