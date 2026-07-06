use embassy_stm32::can::Can;
use embassy_time::Timer;

const TICK_DELAY_MS: u64 = 100;

#[embassy_executor::task]
pub async fn imd_task(can_bus: Can<'static>) {
    loop {
        Timer::after_millis(TICK_DELAY_MS).await;
    }
}
