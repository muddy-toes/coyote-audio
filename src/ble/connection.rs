use std::sync::Arc;
use std::time::Duration;

use btleplug::api::{Characteristic, Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::interval;

use crate::ble::protocol::{Intensity, Waveform};
use crate::ble::{CHAR_PWM_A34, CHAR_PWM_AB2, CHAR_PWM_B34, COMMAND_INTERVAL_MS, SERVICE_UUID};

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

struct CoyoteCharacteristics {
    pwm_ab2: Characteristic,
    pwm_a34: Characteristic,
    pwm_b34: Characteristic,
}

pub struct CoyoteConnection {
    peripheral: Peripheral,
    characteristics: Option<CoyoteCharacteristics>,
    state: Arc<Mutex<ConnectionState>>,
    current_intensity: Arc<Mutex<Intensity>>,
    current_waveform_a: Arc<Mutex<Waveform>>,
    current_waveform_b: Arc<Mutex<Waveform>>,
}

impl CoyoteConnection {
    pub fn new(peripheral: Peripheral) -> Self {
        Self {
            peripheral,
            characteristics: None,
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            current_intensity: Arc::new(Mutex::new(Intensity::default())),
            current_waveform_a: Arc::new(Mutex::new(Waveform::default())),
            current_waveform_b: Arc::new(Mutex::new(Waveform::default())),
        }
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

    async fn find_characteristics(&self) -> Result<CoyoteCharacteristics, ConnectionError> {
        let services = self.peripheral.services();

        let service = services
            .iter()
            .find(|s| s.uuid == SERVICE_UUID)
            .ok_or_else(|| ConnectionError::ServiceNotFound(SERVICE_UUID.to_string()))?;

        let pwm_ab2 = service
            .characteristics
            .iter()
            .find(|c| c.uuid == CHAR_PWM_AB2)
            .cloned()
            .ok_or_else(|| ConnectionError::CharacteristicNotFound("PWM_AB2".to_string()))?;

        let pwm_a34 = service
            .characteristics
            .iter()
            .find(|c| c.uuid == CHAR_PWM_A34)
            .cloned()
            .ok_or_else(|| ConnectionError::CharacteristicNotFound("PWM_A34".to_string()))?;

        let pwm_b34 = service
            .characteristics
            .iter()
            .find(|c| c.uuid == CHAR_PWM_B34)
            .cloned()
            .ok_or_else(|| ConnectionError::CharacteristicNotFound("PWM_B34".to_string()))?;

        Ok(CoyoteCharacteristics {
            pwm_ab2,
            pwm_a34,
            pwm_b34,
        })
    }

    pub async fn set_intensity(&self, intensity: Intensity) -> Result<(), ConnectionError> {
        let chars = self
            .characteristics
            .as_ref()
            .ok_or(ConnectionError::NotConnected)?;

        let data = intensity.encode();
        self.peripheral
            .write(&chars.pwm_ab2, &data, WriteType::WithoutResponse)
            .await?;

        {
            let mut current = self.current_intensity.lock().await;
            *current = intensity;
        }

        Ok(())
    }

    pub async fn set_waveform_a(&self, waveform: Waveform) -> Result<(), ConnectionError> {
        let chars = self
            .characteristics
            .as_ref()
            .ok_or(ConnectionError::NotConnected)?;

        let data = waveform.encode();
        self.peripheral
            .write(&chars.pwm_a34, &data, WriteType::WithoutResponse)
            .await?;

        {
            let mut current = self.current_waveform_a.lock().await;
            *current = waveform;
        }

        Ok(())
    }

    pub async fn set_waveform_b(&self, waveform: Waveform) -> Result<(), ConnectionError> {
        let chars = self
            .characteristics
            .as_ref()
            .ok_or(ConnectionError::NotConnected)?;

        let data = waveform.encode();
        self.peripheral
            .write(&chars.pwm_b34, &data, WriteType::WithoutResponse)
            .await?;

        {
            let mut current = self.current_waveform_b.lock().await;
            *current = waveform;
        }

        Ok(())
    }

    pub async fn start_keepalive_loop(&self) -> Result<(), ConnectionError> {
        let mut ticker = interval(Duration::from_millis(COMMAND_INTERVAL_MS));

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

            if let Some(chars) = &self.characteristics {
                self.peripheral
                    .write(&chars.pwm_ab2, &intensity.encode(), WriteType::WithoutResponse)
                    .await?;
                self.peripheral
                    .write(&chars.pwm_a34, &waveform_a.encode(), WriteType::WithoutResponse)
                    .await?;
                self.peripheral
                    .write(&chars.pwm_b34, &waveform_b.encode(), WriteType::WithoutResponse)
                    .await?;
            }
        }
    }
}
