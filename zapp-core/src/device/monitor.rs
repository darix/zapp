//! Continuous device monitoring via USB hotplug events.
//!
//! Unlike `watcher` which blocks until a single bootloader appears,
//! `DeviceMonitor` provides a stream of connect/disconnect events
//! for both normal keyboards and bootloader devices.

use std::sync::mpsc;
use std::thread;

use futures_lite::StreamExt;
use nusb::hotplug::HotplugEvent;
use nusb::{DeviceId, MaybeFuture};

use super::ids::{
    identify_bootloader, identify_keyboard, keyboard_for_bootloader, BootloaderKind, Keyboard,
};
use crate::ZappError;

/// Events emitted by the device monitor.
#[derive(Debug)]
pub enum DeviceEvent {
    /// A ZSA keyboard was connected in normal mode.
    KeyboardConnected {
        keyboard: Keyboard,
        serial: String,
        pid: u16,
    },
    /// A ZSA keyboard was disconnected.
    KeyboardDisconnected,
    /// A bootloader device appeared.
    BootloaderConnected {
        keyboard: Option<Keyboard>,
        kind: BootloaderKind,
        vid: u16,
        pid: u16,
    },
    /// A bootloader device was disconnected.
    BootloaderDisconnected,
}

enum TrackedDevice {
    Keyboard,
    Bootloader,
}

/// Monitors USB for ZSA device connect/disconnect events using hotplug.
pub struct DeviceMonitor {
    rx: mpsc::Receiver<DeviceEvent>,
}

impl DeviceMonitor {
    /// Start monitoring. Performs an initial scan, then watches for hotplug events.
    pub fn start() -> Result<Self, ZappError> {
        let (tx, rx) = mpsc::channel();

        // Start hotplug watcher before initial scan to avoid missing events
        let watch = nusb::watch_devices()?;

        // Initial scan for already-connected devices
        for dev_info in nusb::list_devices().wait()? {
            let vid = dev_info.vendor_id();
            let pid = dev_info.product_id();

            if let Some(keyboard) = identify_keyboard(vid, pid) {
                let serial = dev_info.serial_number().unwrap_or_default().to_string();
                let _ = tx.send(DeviceEvent::KeyboardConnected {
                    keyboard,
                    serial,
                    pid,
                });
            } else if let Some(kind) = identify_bootloader(vid, pid) {
                let keyboard = keyboard_for_bootloader(vid, pid);
                let _ = tx.send(DeviceEvent::BootloaderConnected {
                    keyboard,
                    kind,
                    vid,
                    pid,
                });
            }
        }

        // Background thread for hotplug events
        thread::spawn(move || {
            // Track connected ZSA devices by their DeviceId so we can identify disconnects
            let mut tracked: Vec<(DeviceId, TrackedDevice)> = Vec::new();

            // Build initial tracked list
            if let Ok(devices) = nusb::list_devices().wait() {
                for dev_info in devices {
                    let vid = dev_info.vendor_id();
                    let pid = dev_info.product_id();
                    if identify_keyboard(vid, pid).is_some() {
                        tracked.push((dev_info.id(), TrackedDevice::Keyboard));
                    } else if identify_bootloader(vid, pid).is_some() {
                        tracked.push((dev_info.id(), TrackedDevice::Bootloader));
                    }
                }
            }

            futures_lite::future::block_on(async {
                let mut watch = std::pin::pin!(watch);

                loop {
                    let Some(event) = watch.next().await else {
                        break;
                    };

                    match event {
                        HotplugEvent::Connected(dev_info) => {
                            let vid = dev_info.vendor_id();
                            let pid = dev_info.product_id();

                            if let Some(keyboard) = identify_keyboard(vid, pid) {
                                tracked.push((dev_info.id(), TrackedDevice::Keyboard));
                                let serial =
                                    dev_info.serial_number().unwrap_or_default().to_string();
                                let _ = tx.send(DeviceEvent::KeyboardConnected {
                                    keyboard,
                                    serial,
                                    pid,
                                });
                            } else if let Some(kind) = identify_bootloader(vid, pid) {
                                tracked.push((dev_info.id(), TrackedDevice::Bootloader));
                                let keyboard = keyboard_for_bootloader(vid, pid);
                                let _ = tx.send(DeviceEvent::BootloaderConnected {
                                    keyboard,
                                    kind,
                                    vid,
                                    pid,
                                });
                            }
                        }
                        HotplugEvent::Disconnected(device_id) => {
                            if let Some(pos) =
                                tracked.iter().position(|(id, _)| *id == device_id)
                            {
                                let (_, kind) = tracked.remove(pos);
                                let _ = tx.send(match kind {
                                    TrackedDevice::Keyboard => DeviceEvent::KeyboardDisconnected,
                                    TrackedDevice::Bootloader => {
                                        DeviceEvent::BootloaderDisconnected
                                    }
                                });
                            }
                        }
                    }
                }
            });
        });

        Ok(Self { rx })
    }

    /// Try to receive the next event without blocking.
    pub fn try_recv(&self) -> Option<DeviceEvent> {
        self.rx.try_recv().ok()
    }

    /// Block until the next event is available.
    pub fn recv(&self) -> Option<DeviceEvent> {
        self.rx.recv().ok()
    }
}
