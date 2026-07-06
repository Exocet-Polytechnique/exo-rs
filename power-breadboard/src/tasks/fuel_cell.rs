use embassy_stm32::{mode::Async, usart::Uart};
use embassy_time::Timer;

const TICK_DELAY_MS: u64 = 100;

#[embassy_executor::task]
pub async fn fuel_cell_task(uart: Uart<'static, Async>) {
    loop {
        Timer::after_millis(TICK_DELAY_MS).await;
    }
}
