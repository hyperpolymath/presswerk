// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Platform-agnostic trait definitions for native capabilities.
//
// Print Doctor supports every conceivable connection type. The bridge traits
// provide abstractions for platform-specific implementations.

use presswerk_core::error::Result;

/// Unified bridge that groups all native capabilities.
///
/// Every connection type from USB to Li-Fi is represented as a trait bound.
/// Platforms that lack a transport (e.g. no FireWire on phones) return
/// `PresswerkError::PlatformUnavailable` from the stub implementation.
pub trait PlatformBridge:
    NativePrint
    + NativeCamera
    + NativeFilePicker
    + NativeKeychain
    + NativeShare
    + NativeUsbPrint
    + NativeBluetoothPrint
    + NativeNfcPrint
    + NativeConnectivity
    + NativeFireWirePrint
    + NativeLightningPrint
    + NativeThunderboltPrint
    + NativeSerialPrint
    + NativeParallelPrint
    + NativeInfraredPrint
    + NativeIBeaconDiscover
    + NativeLiFiPrint
    + NativeUsbDrivePrint
{
    /// Human-readable platform name (e.g. "iOS 17", "Android 14").
    fn platform_name(&self) -> &str;
}

/// Send documents to the OS-level print dialog.
pub trait NativePrint {
    /// Open the native print dialog for the given document bytes.
    /// Returns Ok(()) if the dialog was presented (user may still cancel).
    fn show_print_dialog(&self, document: &[u8], mime_type: &str) -> Result<()>;
}

/// Capture images from the device camera.
pub trait NativeCamera {
    /// Launch the system camera and return the captured JPEG bytes.
    /// Returns Ok(None) if the user cancelled.
    fn capture_image(&self) -> Result<Option<Vec<u8>>>;
}

/// Pick files from the device storage.
pub trait NativeFilePicker {
    /// Show a file picker filtered to the given MIME types.
    /// Returns the file path chosen, or None if cancelled.
    fn pick_file(&self, mime_types: &[&str]) -> Result<Option<String>>;

    /// Read the bytes of a previously picked file.
    fn read_picked_file(&self, path: &str) -> Result<Vec<u8>>;
}

/// Secure key storage in the platform keychain / keystore.
pub trait NativeKeychain {
    /// Store a secret under the given key.
    fn store_secret(&self, key: &str, value: &[u8]) -> Result<()>;

    /// Retrieve a secret by key. Returns None if not found.
    fn load_secret(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Delete a secret by key.
    fn delete_secret(&self, key: &str) -> Result<()>;
}

/// Share content via the OS share sheet.
pub trait NativeShare {
    /// Share a file with other apps via the native share sheet.
    fn share_file(&self, path: &str, mime_type: &str) -> Result<()>;

    /// Share text content (e.g. diagnostic report summary).
    fn share_text(&self, text: &str) -> Result<()>;
}

/// Print via USB connection (OTG on mobile, direct on desktop).
pub trait NativeUsbPrint {
    /// Detect USB-connected printers.
    fn detect_usb_printers(&self) -> Result<Vec<UsbPrinterInfo>>;

    /// Send document bytes to a USB printer.
    fn print_usb(&self, device_id: &str, document: &[u8], mime_type: &str) -> Result<()>;
}

/// Print via Bluetooth (classic SPP or BLE).
pub trait NativeBluetoothPrint {
    /// Scan for Bluetooth printers.
    fn scan_bluetooth_printers(&self) -> Result<Vec<BluetoothPrinterInfo>>;

    /// Send document bytes to a Bluetooth printer.
    fn print_bluetooth(&self, device_id: &str, document: &[u8]) -> Result<()>;
}

/// NFC tag-based printer connection (tap to connect).
pub trait NativeNfcPrint {
    /// Read an NFC tag for printer connection info.
    fn read_nfc_printer_tag(&self) -> Result<Option<NfcPrinterInfo>>;
}

/// Network connectivity information.
pub trait NativeConnectivity {
    /// Get the current Wi-Fi network name (SSID).
    fn wifi_ssid(&self) -> Result<Option<String>>;

    /// Whether Wi-Fi Direct is available.
    fn supports_wifi_direct(&self) -> bool;

    /// Discover printers via Wi-Fi Direct.
    fn discover_wifi_direct_printers(&self) -> Result<Vec<WifiDirectPrinterInfo>>;
}

/// Print via FireWire (IEEE 1394) — legacy high-speed connection.
pub trait NativeFireWirePrint {
    /// Detect FireWire-connected printers.
    fn detect_firewire_printers(&self) -> Result<Vec<FireWirePrinterInfo>>;

    /// Send document bytes to a FireWire printer.
    fn print_firewire(&self, device_id: &str, document: &[u8], mime_type: &str) -> Result<()>;
}

/// Print via Apple Lightning connector.
pub trait NativeLightningPrint {
    /// Detect Lightning-connected printers (via MFi accessories).
    fn detect_lightning_printers(&self) -> Result<Vec<LightningPrinterInfo>>;

    /// Send document bytes to a Lightning-connected printer.
    fn print_lightning(&self, device_id: &str, document: &[u8], mime_type: &str) -> Result<()>;
}

/// Print via Thunderbolt connection (USB-C/Thunderbolt 3/4).
pub trait NativeThunderboltPrint {
    /// Detect Thunderbolt-connected printers.
    fn detect_thunderbolt_printers(&self) -> Result<Vec<ThunderboltPrinterInfo>>;

    /// Send document bytes to a Thunderbolt printer.
    fn print_thunderbolt(&self, device_id: &str, document: &[u8], mime_type: &str) -> Result<()>;
}

/// Print via RS-232 serial port (DB-9/DB-25).
pub trait NativeSerialPrint {
    /// Detect serial-connected printers.
    fn detect_serial_printers(&self) -> Result<Vec<SerialPrinterInfo>>;

    /// Send document bytes over RS-232.
    fn print_serial(&self, port: &str, baud_rate: u32, document: &[u8]) -> Result<()>;
}

/// Print via parallel port (LPT / IEEE 1284 / Centronics).
pub trait NativeParallelPrint {
    /// Detect parallel-port printers.
    fn detect_parallel_printers(&self) -> Result<Vec<ParallelPrinterInfo>>;

    /// Send document bytes to a parallel printer.
    fn print_parallel(&self, port: &str, document: &[u8]) -> Result<()>;
}

/// Print via IrDA (infrared data association).
pub trait NativeInfraredPrint {
    /// Scan for infrared-capable printers.
    fn scan_infrared_printers(&self) -> Result<Vec<InfraredPrinterInfo>>;

    /// Send document bytes over IrDA.
    fn print_infrared(&self, device_id: &str, document: &[u8]) -> Result<()>;
}

/// Discover printers via iBeacon proximity.
///
/// iBeacon is used for discovery only — the actual print transport uses
/// another protocol (typically Wi-Fi or Bluetooth).
pub trait NativeIBeaconDiscover {
    /// Scan for printer iBeacon advertisements.
    fn scan_ibeacon_printers(&self) -> Result<Vec<IBeaconPrinterInfo>>;
}

/// Print via Li-Fi (light fidelity — visible light communication).
pub trait NativeLiFiPrint {
    /// Detect Li-Fi endpoints (experimental — requires hardware).
    fn detect_lifi_endpoints(&self) -> Result<Vec<LiFiEndpointInfo>>;

    /// Send document bytes over Li-Fi link.
    fn print_lifi(&self, endpoint_id: &str, document: &[u8]) -> Result<()>;
}

/// Print via USB mass storage (sneakernet — copy to USB stick, walk to printer).
pub trait NativeUsbDrivePrint {
    /// Detect mounted USB drives.
    fn detect_usb_drives(&self) -> Result<Vec<UsbDriveInfo>>;

    /// Copy a document to a USB drive for manual delivery to a printer.
    fn copy_to_usb_drive(&self, drive_id: &str, document: &[u8], filename: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Info structs for each connection type
// ---------------------------------------------------------------------------

/// USB printer information.
#[derive(Debug, Clone)]
pub struct UsbPrinterInfo {
    pub device_id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
}

/// Bluetooth printer information.
#[derive(Debug, Clone)]
pub struct BluetoothPrinterInfo {
    pub device_id: String,
    pub name: String,
    pub is_ble: bool,
}

/// NFC tag printer connection info.
#[derive(Debug, Clone)]
pub struct NfcPrinterInfo {
    pub uri: String,
    pub name: Option<String>,
}

/// Wi-Fi Direct printer information.
#[derive(Debug, Clone)]
pub struct WifiDirectPrinterInfo {
    pub device_name: String,
    pub device_address: String,
}

/// FireWire (IEEE 1394) printer information.
#[derive(Debug, Clone)]
pub struct FireWirePrinterInfo {
    pub device_id: String,
    pub name: String,
    pub node_id: u16,
}

/// Lightning (MFi) printer information.
#[derive(Debug, Clone)]
pub struct LightningPrinterInfo {
    pub device_id: String,
    pub name: String,
    pub protocol_name: String,
}

/// Thunderbolt printer information.
#[derive(Debug, Clone)]
pub struct ThunderboltPrinterInfo {
    pub device_id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
}

/// RS-232 serial port printer information.
#[derive(Debug, Clone)]
pub struct SerialPrinterInfo {
    pub port: String,
    pub name: Option<String>,
    pub baud_rate: u32,
}

/// Parallel port (LPT/IEEE 1284) printer information.
#[derive(Debug, Clone)]
pub struct ParallelPrinterInfo {
    pub port: String,
    pub name: Option<String>,
    pub device_id_string: Option<String>,
}

/// IrDA (infrared) printer information.
#[derive(Debug, Clone)]
pub struct InfraredPrinterInfo {
    pub device_id: String,
    pub name: String,
}

/// iBeacon printer advertisement.
#[derive(Debug, Clone)]
pub struct IBeaconPrinterInfo {
    pub uuid: String,
    pub major: u16,
    pub minor: u16,
    pub name: Option<String>,
    /// The URI to connect to once discovered (usually IPP over Wi-Fi).
    pub printer_uri: Option<String>,
}

/// Li-Fi endpoint information.
#[derive(Debug, Clone)]
pub struct LiFiEndpointInfo {
    pub endpoint_id: String,
    pub name: Option<String>,
}

/// USB mass storage drive information.
#[derive(Debug, Clone)]
pub struct UsbDriveInfo {
    pub drive_id: String,
    pub label: Option<String>,
    pub mount_point: String,
    pub free_bytes: u64,
}
