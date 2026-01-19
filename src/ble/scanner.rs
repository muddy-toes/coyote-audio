use std::time::Duration;

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceVersion {
    V2,
    V3,
}

impl std::fmt::Display for DeviceVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceVersion::V2 => write!(f, "V2"),
            DeviceVersion::V3 => write!(f, "V3"),
        }
    }
}

pub const DEVICE_NAME_V2: &str = "D-LAB ESTIM01";
pub const DEVICE_NAME_V3: &str = "47L121000";

#[derive(Error, Debug)]
pub enum ScannerError {
    #[error("Bluetooth error: {0}")]
    Bluetooth(#[from] btleplug::Error),
    #[error("No Bluetooth adapter found")]
    NoAdapter,
}

#[derive(Debug, Clone)]
pub struct CoyoteDevice {
    pub name: String,
    pub address: String,
    pub peripheral: Peripheral,
    pub version: DeviceVersion,
}

impl CoyoteDevice {
    pub fn new(name: String, address: String, peripheral: Peripheral, version: DeviceVersion) -> Self {
        Self {
            name,
            address,
            peripheral,
            version,
        }
    }
}

pub struct CoyoteScanner {
    adapter: Adapter,
}

impl CoyoteScanner {
    pub async fn new() -> Result<Self, ScannerError> {
        let manager = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter = adapters.into_iter().next().ok_or(ScannerError::NoAdapter)?;

        Ok(Self { adapter })
    }

    /// Get reference to the BLE adapter
    pub fn adapter(&self) -> &Adapter {
        &self.adapter
    }

    pub async fn start_scan(&self) -> Result<(), ScannerError> {
        self.adapter.start_scan(ScanFilter::default()).await?;
        Ok(())
    }

    pub async fn stop_scan(&self) -> Result<(), ScannerError> {
        self.adapter.stop_scan().await?;
        Ok(())
    }

    pub async fn scan_for_devices(&self, duration: Duration) -> Result<Vec<CoyoteDevice>, ScannerError> {
        self.start_scan().await?;
        tokio::time::sleep(duration).await;
        self.stop_scan().await?;

        self.get_discovered_devices().await
    }

    pub async fn get_discovered_devices(&self) -> Result<Vec<CoyoteDevice>, ScannerError> {
        let peripherals = self.adapter.peripherals().await?;
        let mut devices = Vec::new();

        for peripheral in peripherals {
            if let Some(properties) = peripheral.properties().await? {
                if let Some(name) = properties.local_name {
                    if name == DEVICE_NAME_V2 {
                        let address = peripheral.address().to_string();
                        devices.push(CoyoteDevice::new(name, address, peripheral, DeviceVersion::V2));
                    } else if name == DEVICE_NAME_V3 {
                        let address = peripheral.address().to_string();
                        devices.push(CoyoteDevice::new(name, address, peripheral, DeviceVersion::V3));
                    }
                }
            }
        }

        Ok(devices)
    }

    pub async fn find_device_by_address(&self, address: &str) -> Result<Option<CoyoteDevice>, ScannerError> {
        let devices = self.get_discovered_devices().await?;
        Ok(devices.into_iter().find(|d| d.address == address))
    }
}
