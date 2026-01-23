use thiserror::Error;
use uuid::Uuid;

/// Trait for version-specific Coyote protocol implementations
pub trait CoyoteProtocol: Send + Sync {
    /// Service UUID for this protocol version
    fn service_uuid(&self) -> Uuid;

    /// Write characteristic UUID (primary characteristic)
    fn write_characteristic_uuid(&self) -> Uuid;

    /// Additional characteristic UUIDs (V2 has multiple, V3 has one)
    fn additional_characteristic_uuids(&self) -> Vec<Uuid>;

    /// Encode a command for transmission
    /// Returns bytes to write and which characteristic index to use
    /// Index 0 = write_characteristic_uuid, 1+ = additional_characteristic_uuids
    fn encode_command(
        &mut self,
        intensity_a: u16,
        intensity_b: u16,
        freq_a: u16,
        freq_b: u16,
    ) -> Vec<(usize, Vec<u8>)>;

    /// Command interval in milliseconds
    fn command_interval_ms(&self) -> u64;

    /// Maximum intensity value for this protocol
    fn max_intensity(&self) -> u16;

    /// Maximum frequency value for this protocol
    fn max_frequency(&self) -> u16;

    /// Set the Z value (pulse width) for waveform generation
    /// X is now calculated dynamically based on frequency
    fn set_z_value(&mut self, _z: u8) {}
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Channel A intensity {0} exceeds maximum 2047")]
    ChannelAIntensityOutOfRange(u16),
    #[error("Channel B intensity {0} exceeds maximum 2047")]
    ChannelBIntensityOutOfRange(u16),
    #[error("Waveform X value {0} exceeds maximum 31")]
    WaveformXOutOfRange(u8),
    #[error("Waveform Y value {0} exceeds maximum 1023")]
    WaveformYOutOfRange(u16),
    #[error("Waveform Z value {0} exceeds maximum 31")]
    WaveformZOutOfRange(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    A,
    B,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Intensity {
    pub channel_a: u16,
    pub channel_b: u16,
}

impl Intensity {
    pub const MAX: u16 = 2047;

    pub fn new(channel_a: u16, channel_b: u16) -> Result<Self, ProtocolError> {
        if channel_a > Self::MAX {
            return Err(ProtocolError::ChannelAIntensityOutOfRange(channel_a));
        }
        if channel_b > Self::MAX {
            return Err(ProtocolError::ChannelBIntensityOutOfRange(channel_b));
        }
        Ok(Self { channel_a, channel_b })
    }

    pub fn encode(&self) -> [u8; 3] {
        // PWM_AB2: 3 bytes total (24 bits)
        // bits 21-11 = Channel A (11 bits, 0-2047)
        // bits 10-0 = Channel B (11 bits, 0-2047)
        //
        // Layout (MSB first in the 24-bit value):
        // [unused:2][A:11][B:11]
        //
        // Byte layout (little-endian transmission):
        // byte[0] = bits 7-0   (B bits 7-0)
        // byte[1] = bits 15-8  (A bits 4-0, B bits 10-8)
        // byte[2] = bits 23-16 (unused:2, A bits 10-5)
        let combined: u32 = ((self.channel_a as u32) << 11) | (self.channel_b as u32);
        [
            (combined & 0xFF) as u8,
            ((combined >> 8) & 0xFF) as u8,
            ((combined >> 16) & 0xFF) as u8,
        ]
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WaveformParams {
    pub x: u8,
    pub y: u16,
    pub z: u8,
}

impl WaveformParams {
    pub const X_MAX: u8 = 31;
    pub const Y_MAX: u16 = 1023;
    pub const Z_MAX: u8 = 31;

    pub fn new(x: u8, y: u16, z: u8) -> Result<Self, ProtocolError> {
        if x > Self::X_MAX {
            return Err(ProtocolError::WaveformXOutOfRange(x));
        }
        if y > Self::Y_MAX {
            return Err(ProtocolError::WaveformYOutOfRange(y));
        }
        if z > Self::Z_MAX {
            return Err(ProtocolError::WaveformZOutOfRange(z));
        }
        Ok(Self { x, y, z })
    }

    pub fn x_ms(&self) -> f32 {
        self.x as f32
    }

    pub fn y_ms(&self) -> f32 {
        self.y as f32
    }

    pub fn z_us(&self) -> f32 {
        self.z as f32 * 5.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Waveform {
    pub params: WaveformParams,
}

impl Waveform {
    pub fn new(params: WaveformParams) -> Self {
        Self { params }
    }

    pub fn encode(&self) -> [u8; 3] {
        // PWM_A34/PWM_B34: 3 bytes total (24 bits)
        // X: bits 4-0 (5 bits, 0-31ms)
        // Y: bits 14-5 (10 bits, 0-1023ms)
        // Z: bits 19-15 (5 bits, pulse width * 5us)
        //
        // Layout (bit positions):
        // [unused:4][Z:5][Y:10][X:5]
        //
        // Byte layout (little-endian):
        // byte[0] = bits 7-0   (Y bits 2-0, X bits 4-0)
        // byte[1] = bits 15-8  (Z bits 0, Y bits 9-3)
        // byte[2] = bits 23-16 (unused:4, Z bits 4-1)
        let x = self.params.x as u32;
        let y = self.params.y as u32;
        let z = self.params.z as u32;

        let combined: u32 = x | (y << 5) | (z << 15);
        [
            (combined & 0xFF) as u8,
            ((combined >> 8) & 0xFF) as u8,
            ((combined >> 16) & 0xFF) as u8,
        ]
    }
}

/// Protocol implementation for Coyote V2 devices
pub struct ProtocolV2 {
    z_value: u8,
}

impl Default for ProtocolV2 {
    fn default() -> Self {
        Self {
            z_value: 20, // Wide pulses (100µs) feel more continuous
        }
    }
}

impl CoyoteProtocol for ProtocolV2 {
    fn service_uuid(&self) -> Uuid {
        Uuid::from_u128(0x955a180b_0fe2_f5aa_a094_84b8d4f3e8ad)
    }

    fn write_characteristic_uuid(&self) -> Uuid {
        // PWM_AB2
        Uuid::from_u128(0x955a1504_0fe2_f5aa_a094_84b8d4f3e8ad)
    }

    fn additional_characteristic_uuids(&self) -> Vec<Uuid> {
        vec![
            Uuid::from_u128(0x955a1505_0fe2_f5aa_a094_84b8d4f3e8ad), // PWM_A34
            Uuid::from_u128(0x955a1506_0fe2_f5aa_a094_84b8d4f3e8ad), // PWM_B34
        ]
    }

    fn encode_command(
        &mut self,
        intensity_a: u16,
        intensity_b: u16,
        freq_a: u16,
        freq_b: u16,
    ) -> Vec<(usize, Vec<u8>)> {
        let intensity =
            Intensity::new(intensity_a.min(2047), intensity_b.min(2047)).unwrap_or_default();
        let waveform_a = self.calculate_waveform(freq_a);
        let waveform_b = self.calculate_waveform(freq_b);

        vec![
            (0, intensity.encode().to_vec()),  // PWM_AB2 - intensity for both channels
            (1, waveform_b.encode().to_vec()), // PWM_A34 = "B通道波形数据" = Channel B waveform
            (2, waveform_a.encode().to_vec()), // PWM_B34 = "A通道波形数据" = Channel A waveform
        ]
    }

    fn command_interval_ms(&self) -> u64 {
        100
    }

    fn max_intensity(&self) -> u16 {
        2047
    }

    fn max_frequency(&self) -> u16 {
        100 // We use 10-100 range
    }

    fn set_z_value(&mut self, z: u8) {
        self.z_value = z.clamp(1, 31);
    }
}

impl ProtocolV2 {
    fn calculate_waveform(&self, coyote_freq: u16) -> Waveform {
        // XToys formula: x = 15 * sqrt(period/1000), y = period - x
        // coyote_freq is the period in ms (10-100 range = 100Hz to 10Hz output)
        // This keeps pulse width perceptually consistent across frequencies
        let period = coyote_freq as f32;
        let x = (15.0 * (period / 1000.0).sqrt()).round() as u8;
        let x = x.clamp(1, 31);
        let y = ((period - x as f32).max(0.0).round() as u16).min(1023);
        let z = self.z_value.clamp(5, 31);
        let params = WaveformParams::new(x, y, z).unwrap_or_default();
        Waveform::new(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intensity_encoding_zeros() {
        let intensity = Intensity::new(0, 0).unwrap();
        assert_eq!(intensity.encode(), [0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_intensity_encoding_max_values() {
        let intensity = Intensity::new(2047, 2047).unwrap();
        // A=2047 (0x7FF), B=2047 (0x7FF)
        // combined = (0x7FF << 11) | 0x7FF = 0x3FFFFF
        // byte[0] = 0xFF, byte[1] = 0xFF, byte[2] = 0x3F
        assert_eq!(intensity.encode(), [0xFF, 0xFF, 0x3F]);
    }

    #[test]
    fn test_intensity_encoding_channel_a_only() {
        let intensity = Intensity::new(2047, 0).unwrap();
        // A=2047 (0x7FF), B=0
        // combined = 0x7FF << 11 = 0x3FF800
        // byte[0] = 0x00, byte[1] = 0xF8, byte[2] = 0x3F
        assert_eq!(intensity.encode(), [0x00, 0xF8, 0x3F]);
    }

    #[test]
    fn test_intensity_encoding_channel_b_only() {
        let intensity = Intensity::new(0, 2047).unwrap();
        // A=0, B=2047 (0x7FF)
        // combined = 0x7FF
        // byte[0] = 0xFF, byte[1] = 0x07, byte[2] = 0x00
        assert_eq!(intensity.encode(), [0xFF, 0x07, 0x00]);
    }

    #[test]
    fn test_intensity_encoding_mixed_values() {
        let intensity = Intensity::new(1024, 512).unwrap();
        // A=1024 (0x400), B=512 (0x200)
        // combined = (0x400 << 11) | 0x200 = 0x200200
        // byte[0] = 0x00, byte[1] = 0x02, byte[2] = 0x20
        assert_eq!(intensity.encode(), [0x00, 0x02, 0x20]);
    }

    #[test]
    fn test_intensity_channel_a_out_of_range() {
        let result = Intensity::new(2048, 0);
        assert!(matches!(result, Err(ProtocolError::ChannelAIntensityOutOfRange(2048))));
    }

    #[test]
    fn test_intensity_channel_b_out_of_range() {
        let result = Intensity::new(0, 2048);
        assert!(matches!(result, Err(ProtocolError::ChannelBIntensityOutOfRange(2048))));
    }

    #[test]
    fn test_waveform_encoding_zeros() {
        let waveform = Waveform::new(WaveformParams::new(0, 0, 0).unwrap());
        assert_eq!(waveform.encode(), [0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_waveform_encoding_max_values() {
        let waveform = Waveform::new(WaveformParams::new(31, 1023, 31).unwrap());
        // X=31 (0x1F), Y=1023 (0x3FF), Z=31 (0x1F)
        // combined = 0x1F | (0x3FF << 5) | (0x1F << 15)
        //          = 0x1F | 0x7FE0 | 0xF8000
        //          = 0xFFFFF
        // byte[0] = 0xFF, byte[1] = 0xFF, byte[2] = 0x0F
        assert_eq!(waveform.encode(), [0xFF, 0xFF, 0x0F]);
    }

    #[test]
    fn test_waveform_encoding_x_only() {
        let waveform = Waveform::new(WaveformParams::new(31, 0, 0).unwrap());
        // X=31, Y=0, Z=0
        // combined = 0x1F
        assert_eq!(waveform.encode(), [0x1F, 0x00, 0x00]);
    }

    #[test]
    fn test_waveform_encoding_y_only() {
        let waveform = Waveform::new(WaveformParams::new(0, 1023, 0).unwrap());
        // X=0, Y=1023 (0x3FF), Z=0
        // combined = 0x3FF << 5 = 0x7FE0
        // byte[0] = 0xE0, byte[1] = 0x7F, byte[2] = 0x00
        assert_eq!(waveform.encode(), [0xE0, 0x7F, 0x00]);
    }

    #[test]
    fn test_waveform_encoding_z_only() {
        let waveform = Waveform::new(WaveformParams::new(0, 0, 31).unwrap());
        // X=0, Y=0, Z=31 (0x1F)
        // combined = 0x1F << 15 = 0xF8000
        // byte[0] = 0x00, byte[1] = 0x80, byte[2] = 0x0F
        assert_eq!(waveform.encode(), [0x00, 0x80, 0x0F]);
    }

    #[test]
    fn test_waveform_x_out_of_range() {
        let result = WaveformParams::new(32, 0, 0);
        assert!(matches!(result, Err(ProtocolError::WaveformXOutOfRange(32))));
    }

    #[test]
    fn test_waveform_y_out_of_range() {
        let result = WaveformParams::new(0, 1024, 0);
        assert!(matches!(result, Err(ProtocolError::WaveformYOutOfRange(1024))));
    }

    #[test]
    fn test_waveform_z_out_of_range() {
        let result = WaveformParams::new(0, 0, 32);
        assert!(matches!(result, Err(ProtocolError::WaveformZOutOfRange(32))));
    }

    #[test]
    fn test_waveform_timing_helpers() {
        let params = WaveformParams::new(10, 500, 20).unwrap();
        assert_eq!(params.x_ms(), 10.0);
        assert_eq!(params.y_ms(), 500.0);
        assert_eq!(params.z_us(), 100.0);
    }
}
