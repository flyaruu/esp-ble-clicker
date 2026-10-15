use core::ops::Range;

use crate::button::{ClickerEvents, ClickerReceiver};
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::Timer;
use embedded_storage_async::nor_flash::NorFlash;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{MapConfig, MapStorage, PostcardValue};
use serde::{Deserialize, Serialize};
use trouble_host::prelude::*;

use log::{error, info, warn};

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 4; // Signal + att

// Appearance AD value: Human Interface Device / Keyboard (0x03c1), little-endian.
const KEYBOARD_APPEARANCE: [u8; 2] = appearance::human_interface_device::KEYBOARD.to_le_bytes();

// GATT Server definition
#[gatt_server]
struct Server {
    battery_service: BatteryService,
    hid_service: HidService,
    device_information_service: DeviceInformationService,
}

// Standard boot-compatible keyboard report descriptor. The report is eight bytes:
// modifiers, reserved byte, and six simultaneous key usages.
static DESC: [u8; 63] = [
    0x05, 0x01, // Generic Desktop
    0x09, 0x06, // Keyboard
    0xA1, 0x01, // Application
    0x05, 0x07, // Keyboard/Keypad
    0x19, 0xE0,
    0x29, 0xE7,
    0x15, 0x00,
    0x25, 0x01,
    0x75, 0x01,
    0x95, 0x08,
    0x81, 0x02, // Modifier input
    0x95, 0x01,
    0x75, 0x08,
    0x81, 0x01, // Reserved input
    0x95, 0x05,
    0x75, 0x01,
    0x05, 0x08,
    0x19, 0x01,
    0x29, 0x05,
    0x91, 0x02, // LED output
    0x95, 0x01,
    0x75, 0x03,
    0x91, 0x01, // Padding output
    0x95, 0x06,
    0x75, 0x08,
    0x15, 0x00,
    0x25, 0x65,
    0x05, 0x07,
    0x19, 0x00,
    0x29, 0x65,
    0x81, 0x00, // Key input
    0xC0,
];

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct HidService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03], permissions(encrypted))]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = DESC, permissions(encrypted))]
    pub(crate) report_map: [u8; 63],
    #[characteristic(uuid = "2a4c", write_without_response, permissions(encrypted))]
    pub(crate) hid_control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1, permissions(encrypted))]
    pub(crate) protocol_mode: u8,
    #[descriptor(uuid = "2908", read = encrypted, value = [0u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify, permissions(encrypted))]
    pub(crate) input_keyboard: [u8; 8],
    #[descriptor(uuid = "2908", read = encrypted, value = [0u8, 2u8])]
    #[characteristic(uuid = "2a4d", read, write, write_without_response, permissions(encrypted))]
    pub(crate) output_keyboard: [u8; 1],
    #[characteristic(uuid = "2a22", read, notify, permissions(encrypted))]
    pub(crate) boot_keyboard_input_report: [u8; 8],
    #[characteristic(uuid = "2a32", read, write, write_without_response, permissions(encrypted))]
    pub(crate) boot_keyboard_output_report: [u8; 1],
}

#[gatt_service(uuid = service::DEVICE_INFORMATION)]
struct DeviceInformationService {
    #[characteristic(uuid = "2a50", read, value = [0x01, 0x5e, 0x04, 0x34, 0x12, 0x01, 0x00])]
    pnp_id: [u8; 7],
}

/// Battery service
#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// Battery Level
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level", type = &'static str)]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10, permissions(encrypted))]
    level: u8,
    #[characteristic(uuid = "408813df-5dd4-1f87-ec11-cdb001100000", write, read, notify)]
    status: bool,
}

// #[cfg(feature = "security")]
#[derive(Serialize, Deserialize)]
struct StoredBondInformation(pub(crate) BondInformation);
// #[cfg(feature = "security")]
impl<'a> PostcardValue<'a> for StoredBondInformation {}

/// Run the BLE stack.
pub async fn run<C, S>(
    controller: C,
    storage: &mut S,
    storage_range: Range<u32>,
    clicker_events: ClickerReceiver,
)
where
    C: Controller,
    S: NorFlash,
{
    // Using a fixed "random" address can be useful for testing. In real scenarios, one would
    // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
    let address: Address = Address::random([0xff, 0x8f, 0x08, 0x05, 0xe4, 0xff]);
    info!("Our address = {}", address);

    let mut resources: HostResources<C, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        // This device has no display or input controls. Just Works pairing gives
        // encryption without requiring a passkey/MITM interaction from the host.
        .set_io_capabilities(IoCapabilities::NoInputNoOutput)
        .build();

    let mut map_storage = MapStorage::<(), _, _>::new(storage, MapConfig::new(storage_range), Cache::new_uncached());
    let mut data_buffer = [0; 64];
    let mut bond_stored = match map_storage.fetch_item(&mut data_buffer, &()).await {
        Ok(Some(StoredBondInformation(bond_info))) => {
            match stack.add_bond_information(bond_info) {
                Ok(()) => {
                    info!("Bond stored. Added it to the BLE stack.");
                    true
                }
                Err(e) => {
                    warn!("Stored bond could not be restored: {:?}", e);
                    false
                }
            }
        }
        Ok(None) => {
            info!("No stored bond.");
            false
        }
        Err(e) => {
            warn!("Could not read stored bond: {:?}", e);
            false
        }
    };

    let runner = stack.runner();
    let mut peripheral = stack.peripheral();

    info!("Starting advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "TrouBLE Keyboard",
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap();

    let link_encrypted = AtomicBool::new(false);

    let _ = join(ble_task(runner), async {
        loop {
            match advertise("TrouBLE Keyboard", &mut peripheral, &server).await {
                Ok(conn) => {
                    conn.raw().set_bondable(true).unwrap();
                    if let Err(e) = conn.raw().request_security() {
                        warn!("[gatt] failed to request security: {:?}", e);
                    }
                    // set up tasks when the connection is established to a central, so they don't run when no one is connected.
                    link_encrypted.store(false, Ordering::Relaxed);
                    let a = gatt_events_task(&mut map_storage, &mut data_buffer, &server, &conn, &mut bond_stored, &link_encrypted);
                    let b = custom_task(&server, &conn, &link_encrypted, clicker_events);
                    // run until any task ends (usually because the connection has been closed),
                    // then return to advertising state.
                    select(a, b).await;
                    info!("Connection dropped");
                }
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    panic!("[adv] error: {:?}", e);
                }
            }
        }
    })
    .await;
}

/// This is a background task that is required to run forever alongside any other BLE tasks.
///
/// ## Alternative
///
/// If you didn't require this to be generic for your application, you could statically spawn this with i.e.
///
/// ```rust,ignore
///
/// #[embassy_executor::task]
/// async fn ble_task(mut runner: Runner<'static, SoftdeviceController<'static>>) {
///     runner.run().await;
/// }
///
/// spawner.must_spawn(ble_task(runner));
/// ```
async fn ble_task<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            #[cfg(feature = "defmt")]
            let e = defmt::Debug2Format(&e);
            panic!("[ble_task] error: {:?}", e);
        }
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task<S: NorFlash>(
    map_storage: &mut MapStorage<(), S, Cache<Uncached, Uncached, Uncached>>,
    data_buffer: &mut [u8],
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    bond_stored: &mut bool,
    link_encrypted: &AtomicBool,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,

            GattConnectionEvent::PassKeyDisplay(passkey) => {
                info!("[gatt] passkey display: {:?}", passkey);
                // Show this to user on device UI/serial if needed.
            }

            GattConnectionEvent::PassKeyConfirm(passkey) => {
                info!("[gatt] passkey confirm: {:?}", passkey);
                if let Err(e) = conn.raw().pass_key_confirm() {
                    warn!("[gatt] pass_key_confirm failed: {:?}", e);
                }
            }

            GattConnectionEvent::PassKeyInput => {
                // Replace with user-provided key path if you have one.
                let key: u32 = 123_456;
                info!("[gatt] passkey input requested, entering {}", key);
                if let Err(e) = conn.raw().pass_key_input(key) {
                    warn!("[gatt] pass_key_input failed: {:?}", e);
                }
            }

            GattConnectionEvent::Encrypted { security_level, bond } => {
                info!(
                    "[gatt] encrypted: {:?}, authenticated={}, bonded={}",
                    security_level,
                    security_level.authenticated(),
                    bond.is_some()
                );

                if security_level.encrypted() {
                    // HID permissions require encryption, not MITM authentication.
                    link_encrypted.store(true, Ordering::Relaxed);
                    info!("[gatt] encrypted link established (MITM={})", security_level.authenticated());
                } else {
                    link_encrypted.store(false, Ordering::Relaxed);
                    warn!("[gatt] link is not encrypted yet");
                }
            }
            GattConnectionEvent::PairingComplete { security_level, bond } => {
                info!("[gatt] pairing complete: {:?}", security_level);
                if let Some(bond) = bond {
                    map_storage
                        .store_item(data_buffer, &(), &StoredBondInformation(bond))
                        .await
                        .unwrap();
                    *bond_stored = true;
                    info!("Bond information stored");
                }
            }
            GattConnectionEvent::PairingFailed(err) => {
                error!("[gatt] pairing error: {:?}", err);
            }
            GattConnectionEvent::Gatt { event } => {
                match &event {
                    GattEvent::Read(event) => {
                        if event.handle() == level.handle {
                            let value = conn.get(&level);
                            info!("[gatt] Read Event to Level Characteristic: {:?}", value);
                        }
                    }
                    GattEvent::Write(event) => {
                        if event.handle() == level.handle {
                            event.with_data(|offset, data| {
                                info!("[gatt] Write Event to Level Characteristic at {}: {:?}", offset, data)
                            });
                        }
                    }
                    GattEvent::NotAllowed(event) => {
                        info!("[gatt] Disallowed GATT request to handle: {:?}", event.handle());
                    }
                    _ => (),
                }

                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                }
            }
            _ => {} // ignore other Gatt Connection Events
        }
    };
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let adv_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids16(&[
                service::BATTERY.to_le_bytes(),
                service::HUMAN_INTERFACE_DEVICE.to_le_bytes(),
                service::DEVICE_INFORMATION.to_le_bytes(),
            ]),
            // trouble-host 0.7 has no named Appearance variant, so encode AD
            // type 0x19 explicitly.
            AdStructure::Unknown {
                ty: 0x19,
                data: &KEYBOARD_APPEARANCE,
            },
        ],
        &mut advertiser_data[..],
    )?;
    let mut scan_data = [0; 31];
    let scan_len = AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(name.as_bytes())],
        &mut scan_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..adv_len],
                scan_data: &scan_data[..scan_len],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("[adv] connection established");
    Ok(conn)
}

/// Send a test keyboard key after the encrypted link is ready.
/// This task stops when the connection is closed or report delivery fails.
async fn custom_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    link_encrypted: &AtomicBool,
    clicker_events: ClickerReceiver,
) {
    // Do not try to generate reports until the encrypted GATT link is ready.
    // The delay also gives the host time to discover the HID service and enable
    // notifications.
    if !link_encrypted.load(Ordering::Relaxed) {
        info!("[custom_task] waiting for encrypted link");
        while !link_encrypted.load(Ordering::Relaxed) {
            Timer::after_millis(250).await;
        }
    }
    loop {
        let keycode = match clicker_events.receive().await {
            ClickerEvents::Forward => 79,
            ClickerEvents::Back => 80,
        };

        if send_keyboard_key(server, conn, keycode).await.is_err() {
            info!("[custom_task] error sending keyboard key report");
            break;
        }


    }
}

async fn send_keyboard_report<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    report: [u8; 8],
) -> Result<(), Error> {
    // Report Protocol (1) uses the report characteristic; Boot Protocol (0)
    // uses the boot input characteristic. Only notify the selected report, and
    // only once its CCCD has been enabled by the host.
    if conn
        .get(&server.hid_service.protocol_mode)
        .unwrap_or(1)
        == 0
    {
        let report_characteristic = &server.hid_service.boot_keyboard_input_report;
        if report_characteristic.should_notify(conn) {
            report_characteristic.notify(conn, &report, true).await?;
        }
    } else {
        let report_characteristic = &server.hid_service.input_keyboard;
        if report_characteristic.should_notify(conn) {
            report_characteristic.notify(conn, &report, true).await?;
        }
    }
    Ok(())
}

async fn send_keyboard_key<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    keycode: u8,
) -> Result<(), Error> {
    let press = [0, 0, keycode, 0, 0, 0, 0, 0];
    send_keyboard_report(server, conn, press).await?;
    Timer::after_millis(20).await;
    send_keyboard_report(server, conn, [0; 8]).await?;
    Timer::after_millis(20).await;
    Ok(())
}
