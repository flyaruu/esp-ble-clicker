use embassy_executor::task;
use embassy_futures::select::{select, Either};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Sender,
};
use embassy_time::Timer;
use esp_hal::gpio::Input;
use log::info;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClickerEvents {
    Forward,
    Back,
}

pub type ClickerSender = Sender<'static, CriticalSectionRawMutex, ClickerEvents, 4>;
pub type ClickerReceiver = embassy_sync::channel::Receiver<
    'static,
    CriticalSectionRawMutex,
    ClickerEvents,
    4,
>;

#[task]
pub async fn button_task(
    mut forward: Input<'static>,
    mut back: Input<'static>,
    events: ClickerSender,
) {
    loop {
        info!("ready");
        match select(
            back.wait_for_falling_edge(),
            forward.wait_for_falling_edge(),
        )
        .await
        {
            Either::First(()) => {
                events.send(ClickerEvents::Forward).await;
               info!("forward pressed");
                ClickerEvents::Forward
            },
            Either::Second(()) => {
                events.send(ClickerEvents::Back).await;
                info!("back pressed");
                ClickerEvents::Back
            },
        };
        Timer::after_millis(300).await;
    }
}
