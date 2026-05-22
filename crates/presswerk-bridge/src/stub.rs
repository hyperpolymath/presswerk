// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Stub bridge for desktop/CI builds where native mobile APIs are unavailable.
//
// Every trait method returns `PlatformUnavailable` — real implementations live
// in the `ios` and `android` modules.

use presswerk_core::error::{PresswerkError, Result};

use crate::traits::*;

/// No-op bridge returned on non-mobile platforms.
pub struct StubBridge;

impl PlatformBridge for StubBridge {
    fn platform_name(&self) -> &str {
        "Desktop (stub)"
    }
}

impl NativePrint for StubBridge {
    fn show_print_dialog(&self, _document: &[u8], _mime_type: &str) -> Result<()> {
        tracing::warn!("NativePrint::show_print_dialog called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeCamera for StubBridge {
    fn capture_image(&self) -> Result<Option<Vec<u8>>> {
        tracing::warn!("NativeCamera::capture_image called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeFilePicker for StubBridge {
    fn pick_file(&self, _mime_types: &[&str]) -> Result<Option<String>> {
        tracing::warn!("NativeFilePicker::pick_file called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }

    fn read_picked_file(&self, _path: &str) -> Result<Vec<u8>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeKeychain for StubBridge {
    fn store_secret(&self, _key: &str, _value: &[u8]) -> Result<()> {
        tracing::warn!("NativeKeychain::store_secret called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }

    fn load_secret(&self, _key: &str) -> Result<Option<Vec<u8>>> {
        tracing::warn!("NativeKeychain::load_secret called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }

    fn delete_secret(&self, _key: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeShare for StubBridge {
    fn share_file(&self, _path: &str, _mime_type: &str) -> Result<()> {
        tracing::warn!("NativeShare::share_file called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }

    fn share_text(&self, _text: &str) -> Result<()> {
        tracing::warn!("NativeShare::share_text called on stub bridge");
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeUsbPrint for StubBridge {
    fn detect_usb_printers(&self) -> Result<Vec<UsbPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_usb(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeBluetoothPrint for StubBridge {
    fn scan_bluetooth_printers(&self) -> Result<Vec<BluetoothPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_bluetooth(&self, _device_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeNfcPrint for StubBridge {
    fn read_nfc_printer_tag(&self) -> Result<Option<NfcPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeConnectivity for StubBridge {
    fn wifi_ssid(&self) -> Result<Option<String>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn supports_wifi_direct(&self) -> bool {
        false
    }

    fn discover_wifi_direct_printers(&self) -> Result<Vec<WifiDirectPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeFireWirePrint for StubBridge {
    fn detect_firewire_printers(&self) -> Result<Vec<FireWirePrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_firewire(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeLightningPrint for StubBridge {
    fn detect_lightning_printers(&self) -> Result<Vec<LightningPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_lightning(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeThunderboltPrint for StubBridge {
    fn detect_thunderbolt_printers(&self) -> Result<Vec<ThunderboltPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_thunderbolt(
        &self,
        _device_id: &str,
        _document: &[u8],
        _mime_type: &str,
    ) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeSerialPrint for StubBridge {
    fn detect_serial_printers(&self) -> Result<Vec<SerialPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_serial(&self, _port: &str, _baud_rate: u32, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeParallelPrint for StubBridge {
    fn detect_parallel_printers(&self) -> Result<Vec<ParallelPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_parallel(&self, _port: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeInfraredPrint for StubBridge {
    fn scan_infrared_printers(&self) -> Result<Vec<InfraredPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_infrared(&self, _device_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeIBeaconDiscover for StubBridge {
    fn scan_ibeacon_printers(&self) -> Result<Vec<IBeaconPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeLiFiPrint for StubBridge {
    fn detect_lifi_endpoints(&self) -> Result<Vec<LiFiEndpointInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_lifi(&self, _endpoint_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeUsbDrivePrint for StubBridge {
    fn detect_usb_drives(&self) -> Result<Vec<UsbDriveInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn copy_to_usb_drive(
        &self,
        _drive_id: &str,
        _document: &[u8],
        _filename: &str,
    ) -> Result<String> {
        Err(PresswerkError::PlatformUnavailable)
    }
}
