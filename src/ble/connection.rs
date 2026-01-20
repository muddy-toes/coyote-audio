use std::sync::Arc;
use std::time::Duration;

use btleplug::api::{Central, Characteristic, Peripheral as _, WriteType};
use btleplug::platform::{Adapter, Peripheral};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::protocol::{CoyoteProtocol, ProtocolV2};

// V2 Battery service and characteristic UUIDs
const BATTERY_SERVICE: Uuid = Uuid::from_u128(0x955a180a_0fe2_f5aa_a094_84b8d4f3e8ad);
const CHAR_BATTERY: Uuid = Uuid::from_u128(0x955a1500_0fe2_f5aa_a094_84b8d4f3e8ad);
use super::protocol_v3::ProtocolV3;
use super::scanner::DeviceVersion;

#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("Bluetooth error: {0}")]
    Bluetooth(#[from] btleplug::Error),
    #[error("Service not found: {0}")]
    ServiceNotFound(String),
    #[error("Characteristic not found: {0}")]
    CharacteristicNotFound(String),
    #[error("Not connected")]
    NotConnected,
    #[error("Connection lost")]
    ConnectionLost,
    #[error("Connection timed out after {0} seconds")]
    Timeout(u64),
    #[error("Connection failed after {0} retries: {1}")]
    RetriesExhausted(u32, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

pub struct CoyoteConnection {
    adapter: Adapter,
    device_address: String,
    peripheral: Option<Peripheral>,
    characteristics: Option<Vec<Characteristic>>,
    protocol: Box<dyn CoyoteProtocol>,
    version: DeviceVersion,
    state: Arc<Mutex<ConnectionState>>,
}

impl CoyoteConnection {
    pub fn new(adapter: Adapter, device_address: String, version: DeviceVersion) -> Self {
        let protocol: Box<dyn CoyoteProtocol> = match version {
            DeviceVersion::V2 => Box::new(ProtocolV2::default()),
            DeviceVersion::V3 => Box::new(ProtocolV3::default()),
        };

        Self {
            adapter,
            device_address,
            peripheral: None,
            characteristics: None,
            protocol,
            version,
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
        }
    }

    pub fn version(&self) -> DeviceVersion {
        self.version
    }

    /// Fetch a fresh peripheral reference from the adapter by address.
    /// This avoids stale D-Bus object references that cause "Method Connect doesn't exist" errors.
    async fn fetch_peripheral(&self) -> Result<Peripheral, ConnectionError> {
        let peripherals = self.adapter.peripherals().await?;

        for peripheral in peripherals {
            if peripheral.address().to_string() == self.device_address {
                return Ok(peripheral);
            }
        }

        Err(ConnectionError::ServiceNotFound(format!(
            "Device {} not found - try rescanning",
            self.device_address
        )))
    }

    /// Get reference to connected peripheral, or NotConnected error
    fn peripheral(&self) -> Result<&Peripheral, ConnectionError> {
        self.peripheral.as_ref().ok_or(ConnectionError::NotConnected)
    }

    /// Initialize V2 device by subscribing to notifications and reading battery
    /// This puts the device into "ready" state (solid white LED) before we start sending commands
    async fn initialize_v2(&self) -> Result<(), ConnectionError> {
        let peripheral = self.peripheral()?;
        let services = peripheral.services();

        // Find battery service and characteristic
        if let Some(battery_service) = services.iter().find(|s| s.uuid == BATTERY_SERVICE) {
            if let Some(battery_char) = battery_service
                .characteristics
                .iter()
                .find(|c| c.uuid == CHAR_BATTERY)
            {
                // Subscribe to battery notifications
                if let Err(e) = peripheral.subscribe(battery_char).await {
                    log::warn!("Failed to subscribe to battery: {}", e);
                }

                // Read battery level
                if let Ok(data) = peripheral.read(battery_char).await {
                    if !data.is_empty() {
                        log::info!("Battery level: {}%", data[0]);
                    }
                }
            }
        }

        // Find PWM service and subscribe to PWM_AB2 notifications
        if let Some(pwm_service) = services
            .iter()
            .find(|s| s.uuid == self.protocol.service_uuid())
        {
            if let Some(pwm_ab2) = pwm_service
                .characteristics
                .iter()
                .find(|c| c.uuid == self.protocol.write_characteristic_uuid())
            {
                if let Err(e) = peripheral.subscribe(pwm_ab2).await {
                    log::warn!("Failed to subscribe to PWM_AB2: {}", e);
                }
            }
        }

        // Brief delay to let device settle into ready state
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        Ok(())
    }

    /// Initialize V3 device by sending BF command to set soft limits
    /// Per V3 protocol: this must be sent after every reconnect
    async fn initialize_v3(&self) -> Result<(), ConnectionError> {
        let peripheral = self.peripheral()?;
        let chars = self
            .characteristics
            .as_ref()
            .ok_or(ConnectionError::NotConnected)?;

        // BF command format (7 bytes):
        // [0] = 0xBF (command header)
        // [1] = Channel A soft limit (0-200)
        // [2] = Channel B soft limit (0-200)
        // [3] = Channel A frequency balance (0-255) - "higher = stronger low-freq impact"
        // [4] = Channel B frequency balance (0-255)
        // [5] = Channel A intensity balance (0-255) - "adjusts pulse width"
        // [6] = Channel B intensity balance (0-255)
        //
        // Set soft limits to 200 (max), balance params to 192 (higher for wider pulses,
        // analogous to the Z=20 pulse width fix on V2)
        let bf_cmd = vec![0xBF, 200, 200, 192, 192, 192, 192];

        peripheral
            .write(&chars[0], &bf_cmd, WriteType::WithoutResponse)
            .await?;

        // Brief delay to let device process the command
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        log::info!("V3 device initialized with soft limits set to 200");

        Ok(())
    }

    /// Default connection timeout in seconds
    const CONNECTION_TIMEOUT_SECS: u64 = 15;
    /// Default number of retry attempts
    const MAX_RETRIES: u32 = 2;

    pub async fn connect(&mut self) -> Result<(), ConnectionError> {
        self.connect_with_timeout(Duration::from_secs(Self::CONNECTION_TIMEOUT_SECS))
            .await
    }

    /// Connect with a specified timeout
    pub async fn connect_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<(), ConnectionError> {
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Connecting;
        }

        // Fetch fresh peripheral reference to avoid stale D-Bus object
        let peripheral = self.fetch_peripheral().await?;

        let timeout_secs = timeout.as_secs();

        // Spawn the connection in a separate task so it can be properly aborted
        // if the timeout fires. This is more robust than tokio::time::timeout
        // alone, which may not interrupt blocked system calls.
        let peripheral_clone = peripheral.clone();
        let connect_handle = tokio::spawn(async move {
            peripheral_clone.connect().await?;
            peripheral_clone.discover_services().await?;
            Ok::<_, btleplug::Error>(())
        });

        // Race the connection against the timeout
        let result = tokio::select! {
            res = connect_handle => {
                match res {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(ConnectionError::Bluetooth(e)),
                    Err(e) => Err(ConnectionError::Bluetooth(btleplug::Error::Other(
                        format!("Connection task panicked: {}", e).into()
                    ))),
                }
            }
            _ = tokio::time::sleep(timeout) => {
                Err(ConnectionError::Timeout(timeout_secs))
            }
        };

        match result {
            Ok(()) => {
                // Store the peripheral only on successful connection
                self.peripheral = Some(peripheral);
            }
            Err(e) => {
                // Try to disconnect in case partial connection occurred
                let _ = peripheral.disconnect().await;
                let mut state = self.state.lock().await;
                *state = ConnectionState::Disconnected;
                return Err(e);
            }
        }

        // V2 devices need initialization (subscribe to notifications, read battery)
        // before sending commands to put them in "ready" state
        if matches!(self.version, DeviceVersion::V2) {
            self.initialize_v2().await?;
        }

        let characteristics = self.find_characteristics().await?;
        self.characteristics = Some(characteristics);

        // V3 devices need BF command to set soft limits on connect
        if matches!(self.version, DeviceVersion::V3) {
            self.initialize_v3().await?;
        }

        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Connected;
        }

        Ok(())
    }

    /// Connect with automatic retry on failure
    pub async fn connect_with_retry(&mut self) -> Result<(), ConnectionError> {
        self.connect_with_retry_count(Self::MAX_RETRIES).await
    }

    /// Connect with a specified number of retries
    pub async fn connect_with_retry_count(
        &mut self,
        max_retries: u32,
    ) -> Result<(), ConnectionError> {
        let mut last_error = String::new();

        for attempt in 0..=max_retries {
            if attempt > 0 {
                log::info!(
                    "Connection attempt {} of {} (retrying after failure)",
                    attempt + 1,
                    max_retries + 1
                );
                // Brief delay between retries
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            match self.connect().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_error = e.to_string();
                    log::warn!("Connection attempt {} failed: {}", attempt + 1, e);

                    // Ensure we're disconnected before retrying
                    if let Some(ref peripheral) = self.peripheral {
                        if let Ok(true) = peripheral.is_connected().await {
                            let _ = peripheral.disconnect().await;
                        }
                    }
                    // Clear stale peripheral reference before retry
                    self.peripheral = None;
                }
            }
        }

        Err(ConnectionError::RetriesExhausted(max_retries, last_error))
    }

    pub async fn disconnect(&mut self) -> Result<(), ConnectionError> {
        if let Some(ref peripheral) = self.peripheral {
            peripheral.disconnect().await?;
        }
        self.peripheral = None;
        self.characteristics = None;

        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Disconnected;
        }

        Ok(())
    }

    pub async fn reconnect(&mut self) -> Result<(), ConnectionError> {
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Reconnecting;
        }

        // Disconnect existing peripheral if connected
        if let Some(ref peripheral) = self.peripheral {
            if peripheral.is_connected().await.unwrap_or(false) {
                let _ = peripheral.disconnect().await;
            }
        }
        // Clear stale peripheral reference before reconnecting
        self.peripheral = None;
        self.characteristics = None;

        self.connect().await
    }

    pub async fn is_connected(&self) -> Result<bool, ConnectionError> {
        match &self.peripheral {
            Some(peripheral) => Ok(peripheral.is_connected().await?),
            None => Ok(false),
        }
    }

    pub async fn state(&self) -> ConnectionState {
        *self.state.lock().await
    }

    async fn find_characteristics(&self) -> Result<Vec<Characteristic>, ConnectionError> {
        let peripheral = self.peripheral()?;
        let services = peripheral.services();

        let service = services
            .iter()
            .find(|s| s.uuid == self.protocol.service_uuid())
            .ok_or_else(|| ConnectionError::ServiceNotFound(self.protocol.service_uuid().to_string()))?;

        let mut chars = Vec::new();

        // Find primary write characteristic
        let write_char = service
            .characteristics
            .iter()
            .find(|c| c.uuid == self.protocol.write_characteristic_uuid())
            .cloned()
            .ok_or_else(|| ConnectionError::CharacteristicNotFound("write".to_string()))?;
        chars.push(write_char);

        // Find additional characteristics (V2 has 2 more, V3 has none)
        for uuid in self.protocol.additional_characteristic_uuids() {
            let char = service
                .characteristics
                .iter()
                .find(|c| c.uuid == uuid)
                .cloned()
                .ok_or_else(|| ConnectionError::CharacteristicNotFound(uuid.to_string()))?;
            chars.push(char);
        }

        Ok(chars)
    }

    /// Set waveform parameters (X and Z values) on the protocol
    pub fn set_waveform_params(&mut self, x: u8, z: u8) {
        self.protocol.set_x_value(x);
        self.protocol.set_z_value(z);
    }

    /// Unified command sending using the version-specific protocol
    pub async fn send_command(
        &mut self,
        intensity_a: u16,
        intensity_b: u16,
        freq_a: u16,
        freq_b: u16,
    ) -> Result<(), ConnectionError> {
        let chars = self
            .characteristics
            .as_ref()
            .ok_or(ConnectionError::NotConnected)?;

        let writes = self
            .protocol
            .encode_command(intensity_a, intensity_b, freq_a, freq_b);

        let peripheral = self.peripheral()?;
        for (char_idx, data) in writes {
            if char_idx < chars.len() {
                peripheral
                    .write(&chars[char_idx], &data, WriteType::WithoutResponse)
                    .await?;
            }
        }

        Ok(())
    }
}
