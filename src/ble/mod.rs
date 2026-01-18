pub mod connection;
pub mod protocol;
pub mod scanner;

pub use connection::CoyoteConnection;
pub use protocol::{Channel, Intensity, Waveform, WaveformParams};
pub use scanner::{CoyoteDevice, CoyoteScanner};

use uuid::Uuid;

pub const DEVICE_NAME: &str = "D-LAB ESTIM01";

pub const SERVICE_UUID: Uuid = Uuid::from_u128(0x955a180b_0fe2_f5aa_a094_84b8d4f3e8ad);

pub const CHAR_PWM_AB2: Uuid = Uuid::from_u128(0x955a1504_0fe2_f5aa_a094_84b8d4f3e8ad);
pub const CHAR_PWM_A34: Uuid = Uuid::from_u128(0x955a1505_0fe2_f5aa_a094_84b8d4f3e8ad);
pub const CHAR_PWM_B34: Uuid = Uuid::from_u128(0x955a1506_0fe2_f5aa_a094_84b8d4f3e8ad);

pub const COMMAND_INTERVAL_MS: u64 = 100;
