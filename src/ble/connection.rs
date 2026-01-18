use std::sync::Arc;
use std::time::Duration;

use btleplug::api::{Characteristic, Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::interval;

use super::protocol::{CoyoteProtocol, Intensity, ProtocolV2, Waveform};
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
    peripheral: Peripheral,
    characteristics: Option<Vec<Characteristic>>,
    protocol: Box<dyn CoyoteProtocol>,
    version: DeviceVersion,
    state: Arc<Mutex<ConnectionState>>,
    current_intensity: Arc<Mutex<Intensity>>,
    current_waveform_a: Arc<Mutex<Waveform>>,
    current_waveform_b: Arc<Mutex<Waveform>>,
}

impl CoyoteConnection {
    pub fn new(peripheral: Peripheral, version: DeviceVersion) -> Self {
        let protocol: Box<dyn CoyoteProtocol> = match version {
            DeviceVersion::V2 => Box::new(ProtocolV2::default()),
            DeviceVersion::V3 => Box::new(ProtocolV3::default()),
        };

        Self {
            peripheral,
            characteristics: None,
            protocol,
            version,
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            current_intensity: Arc::new(Mutex::new(Intensity::default())),
            current_waveform_a: Arc::new(Mutex::new(Waveform::default())),
            current_waveform_b: Arc::new(Mutex::new(Waveform::default())),
        }
    }

    pub fn version(&self) -> DeviceVersion {
        self.version
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

        let timeout_secs = timeout.as_secs();

        // Wrap the connection attempt in a timeout
        let connect_future = async {
            self.peripheral.connect().await?;
            self.peripheral.discover_services().await?;
            Ok::<_, ConnectionError>(())
        };

        match tokio::time::timeout(timeout, connect_future).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let mut state = self.state.lock().await;
                *state = ConnectionState::Disconnected;
                return Err(e);
            }
            Err(_) => {
                let mut state = self.state.lock().await;
                *state = ConnectionState::Disconnected;
                return Err(ConnectionError::Timeout(timeout_secs));
            }
        }

        let characteristics = self.find_characteristics().await?;
        self.characteristics = Some(characteristics);

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
                    if let Ok(true) = self.peripheral.is_connected().await {
                        let _ = self.peripheral.disconnect().await;
                    }
                }
            }
        }

        Err(ConnectionError::RetriesExhausted(max_retries, last_error))
    }

    pub async fn disconnect(&mut self) -> Result<(), ConnectionError> {
        self.peripheral.disconnect().await?;
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

        if self.peripheral.is_connected().await? {
            self.peripheral.disconnect().await?;
        }

        self.connect().await
    }

    pub async fn is_connected(&self) -> Result<bool, ConnectionError> {
        Ok(self.peripheral.is_connected().await?)
    }

    pub async fn state(&self) -> ConnectionState {
        *self.state.lock().await
    }

    async fn find_characteristics(&self) -> Result<Vec<Characteristic>, ConnectionError> {
        let services = self.peripheral.services();

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

        for (char_idx, data) in writes {
            if char_idx < chars.len() {
                self.peripheral
                    .write(&chars[char_idx], &data, WriteType::WithoutResponse)
                    .await?;
            }
        }

        // Update cached state for keepalive
        {
            let mut current = self.current_intensity.lock().await;
            *current = Intensity::new(intensity_a.min(2047), intensity_b.min(2047)).unwrap_or_default();
        }

        Ok(())
    }

    /// Set intensity only (V2 backwards compatibility - uses send_command internally)
    pub async fn set_intensity(&mut self, intensity: Intensity) -> Result<(), ConnectionError> {
        // Get current waveform to compute frequency
        let waveform_a = {
            let current = self.current_waveform_a.lock().await;
            *current
        };
        let waveform_b = {
            let current = self.current_waveform_b.lock().await;
            *current
        };

        // Convert waveform back to frequency (x + y)
        let freq_a = waveform_a.params.x as u16 + waveform_a.params.y;
        let freq_b = waveform_b.params.x as u16 + waveform_b.params.y;

        self.send_command(intensity.channel_a, intensity.channel_b, freq_a, freq_b)
            .await?;

        {
            let mut current = self.current_intensity.lock().await;
            *current = intensity;
        }

        Ok(())
    }

    /// Set waveform for channel A (V2 backwards compatibility - uses send_command internally)
    pub async fn set_waveform_a(&mut self, waveform: Waveform) -> Result<(), ConnectionError> {
        let intensity = {
            let current = self.current_intensity.lock().await;
            *current
        };
        let waveform_b = {
            let current = self.current_waveform_b.lock().await;
            *current
        };

        let freq_a = waveform.params.x as u16 + waveform.params.y;
        let freq_b = waveform_b.params.x as u16 + waveform_b.params.y;

        self.send_command(intensity.channel_a, intensity.channel_b, freq_a, freq_b)
            .await?;

        {
            let mut current = self.current_waveform_a.lock().await;
            *current = waveform;
        }

        Ok(())
    }

    /// Set waveform for channel B (V2 backwards compatibility - uses send_command internally)
    pub async fn set_waveform_b(&mut self, waveform: Waveform) -> Result<(), ConnectionError> {
        let intensity = {
            let current = self.current_intensity.lock().await;
            *current
        };
        let waveform_a = {
            let current = self.current_waveform_a.lock().await;
            *current
        };

        let freq_a = waveform_a.params.x as u16 + waveform_a.params.y;
        let freq_b = waveform.params.x as u16 + waveform.params.y;

        self.send_command(intensity.channel_a, intensity.channel_b, freq_a, freq_b)
            .await?;

        {
            let mut current = self.current_waveform_b.lock().await;
            *current = waveform;
        }

        Ok(())
    }

    pub async fn start_keepalive_loop(&mut self) -> Result<(), ConnectionError> {
        let interval_ms = self.protocol.command_interval_ms();
        let mut ticker = interval(Duration::from_millis(interval_ms));

        loop {
            ticker.tick().await;

            if !self.peripheral.is_connected().await? {
                return Err(ConnectionError::ConnectionLost);
            }

            let intensity = {
                let current = self.current_intensity.lock().await;
                *current
            };

            let waveform_a = {
                let current = self.current_waveform_a.lock().await;
                *current
            };

            let waveform_b = {
                let current = self.current_waveform_b.lock().await;
                *current
            };

            let freq_a = waveform_a.params.x as u16 + waveform_a.params.y;
            let freq_b = waveform_b.params.x as u16 + waveform_b.params.y;

            self.send_command(intensity.channel_a, intensity.channel_b, freq_a, freq_b)
                .await?;
        }
    }
}
