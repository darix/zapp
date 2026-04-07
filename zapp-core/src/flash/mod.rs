pub mod dfu;
pub mod halfkay;

use crate::device::ids::{self, Keyboard};
use crate::device::{BootloaderDevice, BootloaderKind};
use crate::firmware::Firmware;
use crate::ZappError;

/// Progress updates emitted during flashing.
#[derive(Debug, Clone)]
pub enum FlashProgress {
    Erasing {
        bytes_erased: usize,
        total_bytes: usize,
    },
    Writing {
        bytes_written: usize,
        total_bytes: usize,
    },
    Resetting,
    Complete,
}

/// Flash a device with the given firmware, reporting progress via callback.
pub fn flash_device(
    device: &BootloaderDevice,
    firmware: &Firmware,
    on_progress: &dyn Fn(FlashProgress),
) -> Result<(), ZappError> {
    validate_firmware_compatibility(device, firmware)?;

    match device.kind {
        BootloaderKind::Stm32Dfu => dfu::flash_dfu(device, firmware, false, on_progress),
        BootloaderKind::IgnitionStm32 | BootloaderKind::IgnitionGd32 => {
            dfu::flash_dfu(device, firmware, true, on_progress)
        }
        BootloaderKind::Halfkay => halfkay::flash_halfkay(device, firmware, on_progress),
    }
}

/// Validate that the firmware is compatible with the target bootloader device.
///
/// Prevents flashing firmware built for one bootloader protocol onto a device
/// using a different protocol (e.g. STM32 DFU firmware onto an Ignition device),
/// which would brick the keyboard due to mismatched base addresses.
fn validate_firmware_compatibility(
    device: &BootloaderDevice,
    firmware: &Firmware,
) -> Result<(), ZappError> {
    let is_ignition = matches!(
        device.kind,
        BootloaderKind::IgnitionStm32 | BootloaderKind::IgnitionGd32
    );

    match firmware {
        Firmware::IgnitionDual { .. } => {
            // Dual-image firmware is only valid for Ignition bootloaders.
            if !is_ignition {
                return Err(ZappError::IncompatibleFirmware {
                    firmware_desc: "Ignition dual-image firmware".into(),
                    device_desc: ids::friendly_name(device.vid, device.pid).into(),
                });
            }
        }
        Firmware::DfuBinary { vid, pid, .. } => {
            if is_ignition && *vid == ids::STM32_VID && *pid == ids::STM32_DFU_PID {
                // Generic STM32 DFU firmware being flashed on an Ignition bootloader.
                // The firmware is linked for 0x0800_0000 but Ignition starts at 0x0800_2000.
                return Err(ZappError::IncompatibleFirmware {
                    firmware_desc: "STM32 DFU firmware".into(),
                    device_desc: ids::friendly_name(device.vid, device.pid).into(),
                });
            }

            // Check for Moonlander cross-revision mismatch using normal-mode PIDs.
            let fw_keyboard = ids::identify_keyboard(*vid, *pid);
            if fw_keyboard == Some(Keyboard::Moonlander) {
                let fw_is_revb = ids::is_moonlander_revb(*pid);
                if fw_is_revb && !is_ignition {
                    return Err(ZappError::IncompatibleFirmware {
                        firmware_desc: "Moonlander rev B (Ignition) firmware".into(),
                        device_desc: ids::friendly_name(device.vid, device.pid).into(),
                    });
                }
                if !fw_is_revb && is_ignition {
                    return Err(ZappError::IncompatibleFirmware {
                        firmware_desc: "Moonlander rev A (STM32 DFU) firmware".into(),
                        device_desc: ids::friendly_name(device.vid, device.pid).into(),
                    });
                }
            }
        }
        Firmware::IntelHex { .. } => {
            // Intel HEX is only valid for HALFKAY devices.
            if device.kind != BootloaderKind::Halfkay {
                return Err(ZappError::IncompatibleFirmware {
                    firmware_desc: "Intel HEX firmware (HALFKAY)".into(),
                    device_desc: ids::friendly_name(device.vid, device.pid).into(),
                });
            }
        }
    }

    Ok(())
}
