#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;

use esp_radio::ble::controller::BleConnector;
use bt_hci::controller::ExternalController;

use embassy_executor::Spawner;
use embedded_storage_async::nor_flash::NorFlash;
use embedded_storage_async::nor_flash::ReadNorFlash;
use embassy_time::Delay;

use log::LevelFilter;

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    log::info!("{}", panic_info);
    loop {}
}

extern crate alloc;



// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.4.0
    // generator parameters: -o esp32c6 -o esp32c6-mini-1 -o unstable-hal -o embassy -o alloc -o ble-trouble -o probe-rs -o agents -o zed -o nightly-aarch64-apple-darwin
    esp_println::logger::init_logger_from_env();

    // rtt_target::rtt_init_print!();
    log::set_max_level(LevelFilter::Info);
    log::info!("info: bla");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // The following pins are used to bootstrap the chip. They are available
                    // for use, but check the datasheet of the module for more information on them.
                    // - GPIO4
// - GPIO5
// - GPIO8
// - GPIO9
// - GPIO15
// These GPIO pins are in use by some feature of the module and should not be used.
                    let _gpio24 = peripherals.GPIO24;
    let _gpio25 = peripherals.GPIO25;
    let _gpio26 = peripherals.GPIO26;
    let _gpio27 = peripherals.GPIO27;
    let _gpio28 = peripherals.GPIO28;
    let _gpio29 = peripherals.GPIO29;
    let _gpio30 = peripherals.GPIO30;


    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    log::info!("Embassy initialized!");


    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 1>::new(transport);

    let flash = esp_storage::FlashStorage::new(peripherals.FLASH);
    let capacity = flash.capacity() as u32;
    let mut flash = embassy_embedded_hal::adapter::BlockingAsync::new(flash);

    // let capacity = <embassy_embedded_hal::adapter::BlockingAsync<_> as ReadNorFlash>::CAPACITY as u32;
    let storage_range = 0..capacity;

    let _ = spawner;
    esp_ble::ble_bas_peripheral_bonding::run(ble_controller, &mut flash, storage_range).await;
    // esp_ble::ble_bas_peripheral_bonding::run(ble_controller, storage_range).await;

    unreachable!();

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.2.2/examples
}
