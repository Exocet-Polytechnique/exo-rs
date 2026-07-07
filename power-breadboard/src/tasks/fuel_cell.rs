use embassy_futures::select::select;
use embassy_stm32::{mode::Async, usart::Uart};
use embassy_time::Timer;

use crate::tasks::can::CURRENT_STATE;

#[embassy_executor::task]
pub async fn fuel_cell_task(uart: Uart<'static, Async>) {
    let mut state_receiver = CURRENT_STATE.receiver().unwrap();

    loop {
        Timer::after_millis(1000).await;
        // match select(state_receiver.changed(), uart.read(buffer)).await {
        // }
    }
}
