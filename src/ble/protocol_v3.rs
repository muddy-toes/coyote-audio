use uuid::Uuid;

use super::protocol::CoyoteProtocol;

/// V3 protocol implementation for Coyote 3.0 ("47L121000")
pub struct ProtocolV3 {
    sequence: u8, // 0-15, wraps around
}

impl Default for ProtocolV3 {
    fn default() -> Self {
        Self { sequence: 0 }
    }
}

impl ProtocolV3 {
    // Base UUID for standard BLE: 0000xxxx-0000-1000-8000-00805f9b34fb
    const BASE_UUID: u128 = 0x00000000_0000_1000_8000_00805f9b34fb;

    fn make_uuid(short: u16) -> Uuid {
        Uuid::from_u128(Self::BASE_UUID | ((short as u128) << 96))
    }

    /// Map V2-style intensity (0-2047) to V3 range (0-200)
    fn map_intensity(v2_intensity: u16) -> u8 {
        ((v2_intensity as u32 * 200) / 2047).min(200) as u8
    }

    /// Map V2-style frequency (10-100) to V3 range (10-240)
    fn map_frequency(v2_freq: u16) -> u8 {
        let clamped = v2_freq.clamp(10, 100);
        let mapped = 10 + ((clamped - 10) as u32 * (240 - 10)) / (100 - 10);
        mapped.min(240) as u8
    }

    fn next_sequence(&mut self) -> u8 {
        let seq = self.sequence;
        self.sequence = (self.sequence + 1) & 0x0F; // Wrap at 16
        seq
    }
}

impl CoyoteProtocol for ProtocolV3 {
    fn service_uuid(&self) -> Uuid {
        Self::make_uuid(0x180C)
    }

    fn write_characteristic_uuid(&self) -> Uuid {
        Self::make_uuid(0x150A)
    }

    fn additional_characteristic_uuids(&self) -> Vec<Uuid> {
        vec![] // V3 uses single combined command
    }

    fn encode_command(
        &mut self,
        intensity_a: u16,
        intensity_b: u16,
        freq_a: u16,
        freq_b: u16,
    ) -> Vec<(usize, Vec<u8>)> {
        // B0 command format (20 bytes):
        // [0] = 0xB0 (command header)
        // [1] = sequence (4 bits) | intensity_mode (4 bits) - use 0b11 for absolute
        // [2] = channel A intensity (0-200)
        // [3] = channel B intensity (0-200)
        // [4-7] = channel A frequencies (4 segments, 10-240 each)
        // [8-11] = channel A waveform intensities (4 segments, 0-100 each)
        // [12-15] = channel B frequencies
        // [16-19] = channel B waveform intensities

        let seq = self.next_sequence();
        let int_a = Self::map_intensity(intensity_a);
        let int_b = Self::map_intensity(intensity_b);
        let freq_a_mapped = Self::map_frequency(freq_a);
        let freq_b_mapped = Self::map_frequency(freq_b);

        // Use constant frequency across all 4 segments (100ms / 4 = 25ms each)
        // Waveform intensity at 100 (max) for all segments
        let mut cmd = vec![0u8; 20];
        cmd[0] = 0xB0;
        cmd[1] = (seq << 4) | 0b0011; // sequence + absolute mode for both channels
        cmd[2] = int_a;
        cmd[3] = int_b;

        // Channel A: 4 frequency bytes, 4 waveform intensity bytes
        for i in 0..4 {
            cmd[4 + i] = freq_a_mapped; // frequencies
            cmd[8 + i] = 100; // waveform intensities (max)
        }

        // Channel B: 4 frequency bytes, 4 waveform intensity bytes
        for i in 0..4 {
            cmd[12 + i] = freq_b_mapped; // frequencies
            cmd[16 + i] = 100; // waveform intensities (max)
        }

        vec![(0, cmd)] // Single write to characteristic 0
    }

    fn command_interval_ms(&self) -> u64 {
        100
    }

    fn max_intensity(&self) -> u16 {
        200
    }

    fn max_frequency(&self) -> u16 {
        240
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_uuid() {
        let proto = ProtocolV3::default();
        // 0x180C in standard BLE base UUID format
        assert_eq!(
            proto.service_uuid(),
            Uuid::from_u128(0x0000180c_0000_1000_8000_00805f9b34fb)
        );
    }

    #[test]
    fn test_write_characteristic_uuid() {
        let proto = ProtocolV3::default();
        // 0x150A in standard BLE base UUID format
        assert_eq!(
            proto.write_characteristic_uuid(),
            Uuid::from_u128(0x0000150a_0000_1000_8000_00805f9b34fb)
        );
    }

    #[test]
    fn test_map_intensity_zero() {
        assert_eq!(ProtocolV3::map_intensity(0), 0);
    }

    #[test]
    fn test_map_intensity_max() {
        assert_eq!(ProtocolV3::map_intensity(2047), 200);
    }

    #[test]
    fn test_map_intensity_mid() {
        // 1024 / 2047 * 200 ≈ 100
        let result = ProtocolV3::map_intensity(1024);
        assert!(result >= 99 && result <= 100);
    }

    #[test]
    fn test_map_frequency_min() {
        assert_eq!(ProtocolV3::map_frequency(10), 10);
    }

    #[test]
    fn test_map_frequency_max() {
        assert_eq!(ProtocolV3::map_frequency(100), 240);
    }

    #[test]
    fn test_map_frequency_mid() {
        // 55 is midpoint of 10-100, should map to midpoint of 10-240 ≈ 125
        let result = ProtocolV3::map_frequency(55);
        assert!(result >= 120 && result <= 130);
    }

    #[test]
    fn test_map_frequency_clamps_low() {
        assert_eq!(ProtocolV3::map_frequency(0), 10);
    }

    #[test]
    fn test_map_frequency_clamps_high() {
        assert_eq!(ProtocolV3::map_frequency(200), 240);
    }

    #[test]
    fn test_sequence_wraps() {
        let mut proto = ProtocolV3::default();
        for i in 0..16 {
            assert_eq!(proto.next_sequence(), i);
        }
        // Should wrap to 0
        assert_eq!(proto.next_sequence(), 0);
    }

    #[test]
    fn test_encode_command_structure() {
        let mut proto = ProtocolV3::default();
        let commands = proto.encode_command(1024, 512, 50, 75);

        assert_eq!(commands.len(), 1);
        let (idx, data) = &commands[0];
        assert_eq!(*idx, 0);
        assert_eq!(data.len(), 20);

        // Check header
        assert_eq!(data[0], 0xB0);
        // First call, sequence = 0, mode = 0b11
        assert_eq!(data[1], 0b0000_0011);
    }

    #[test]
    fn test_encode_command_intensities() {
        let mut proto = ProtocolV3::default();
        let commands = proto.encode_command(2047, 0, 50, 50);
        let data = &commands[0].1;

        assert_eq!(data[2], 200); // max intensity for A
        assert_eq!(data[3], 0); // zero intensity for B
    }

    #[test]
    fn test_encode_command_frequencies() {
        let mut proto = ProtocolV3::default();
        let commands = proto.encode_command(100, 100, 10, 100);
        let data = &commands[0].1;

        // Channel A frequencies (indices 4-7) should all be 10
        for i in 4..8 {
            assert_eq!(data[i], 10);
        }
        // Channel B frequencies (indices 12-15) should all be 240
        for i in 12..16 {
            assert_eq!(data[i], 240);
        }
    }

    #[test]
    fn test_encode_command_waveform_intensities() {
        let mut proto = ProtocolV3::default();
        let commands = proto.encode_command(100, 100, 50, 50);
        let data = &commands[0].1;

        // Channel A waveform intensities (indices 8-11) should all be 100
        for i in 8..12 {
            assert_eq!(data[i], 100);
        }
        // Channel B waveform intensities (indices 16-19) should all be 100
        for i in 16..20 {
            assert_eq!(data[i], 100);
        }
    }

    #[test]
    fn test_sequence_in_encoded_command() {
        let mut proto = ProtocolV3::default();

        let cmd1 = proto.encode_command(100, 100, 50, 50);
        assert_eq!(cmd1[0].1[1] >> 4, 0); // sequence 0

        let cmd2 = proto.encode_command(100, 100, 50, 50);
        assert_eq!(cmd2[0].1[1] >> 4, 1); // sequence 1

        // Skip to sequence 15
        for _ in 0..13 {
            proto.encode_command(100, 100, 50, 50);
        }

        let cmd15 = proto.encode_command(100, 100, 50, 50);
        assert_eq!(cmd15[0].1[1] >> 4, 15); // sequence 15

        let cmd_wrap = proto.encode_command(100, 100, 50, 50);
        assert_eq!(cmd_wrap[0].1[1] >> 4, 0); // wrapped to 0
    }
}
