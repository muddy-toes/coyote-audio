pub mod connection;
pub mod protocol;
pub mod protocol_v3;
pub mod scanner;

pub use connection::CoyoteConnection;
pub use protocol::{Channel, CoyoteProtocol, Intensity, ProtocolV2, Waveform, WaveformParams};
pub use protocol_v3::ProtocolV3;
pub use scanner::{CoyoteDevice, CoyoteScanner, DeviceVersion, DEVICE_NAME_V2, DEVICE_NAME_V3};

use uuid::Uuid;

pub const SERVICE_UUID: Uuid = Uuid::from_u128(0x955a180b_0fe2_f5aa_a094_84b8d4f3e8ad);

pub const CHAR_PWM_AB2: Uuid = Uuid::from_u128(0x955a1504_0fe2_f5aa_a094_84b8d4f3e8ad);
pub const CHAR_PWM_A34: Uuid = Uuid::from_u128(0x955a1505_0fe2_f5aa_a094_84b8d4f3e8ad);
pub const CHAR_PWM_B34: Uuid = Uuid::from_u128(0x955a1506_0fe2_f5aa_a094_84b8d4f3e8ad);

pub const COMMAND_INTERVAL_MS: u64 = 80;  // Send early to interrupt device before its 100ms window expires and it stops
