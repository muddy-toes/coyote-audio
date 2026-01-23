//! Maps audio analysis results to Coyote V2 protocol commands.
//!
//! Amplitude always controls intensity. The detected dominant frequency is mapped
//! to the Coyote's frequency output (X+Y) based on a user-configured frequency band.

use std::time::{Duration, Instant};

use crate::ble::protocol::Intensity;
use crate::config::Config;

use super::AnalysisResult;

/// Configuration for the audio-to-signal mapper
#[derive(Debug, Clone)]
pub struct MapperConfig {
    /// Default frequency when audio frequency is outside configured band (Coyote freq 10-100)
    pub default_coyote_freq: u16,
    /// Soft ramp-up duration
    pub ramp_duration: Duration,
}

impl Default for MapperConfig {
    fn default() -> Self {
        Self {
            default_coyote_freq: 50, // Middle of range when no frequency detected
            ramp_duration: Duration::from_millis(100), // Brief startup ramp only
        }
    }
}

/// Command ready to send to the Coyote device
#[derive(Debug, Clone)]
pub struct CoyoteCommand {
    pub intensity: Intensity,
    /// Coyote frequency for channel A (10-100, period in ms)
    pub freq_a: u16,
    /// Coyote frequency for channel B (10-100, period in ms)
    pub freq_b: u16,
}

impl Default for CoyoteCommand {
    fn default() -> Self {
        Self {
            intensity: Intensity::default(),
            freq_a: 50,
            freq_b: 50,
        }
    }
}

/// State for soft ramp-up tracking
#[derive(Debug)]
struct RampState {
    start_time: Option<Instant>,
    last_intensity_a: u16,
    last_intensity_b: u16,
}

impl Default for RampState {
    fn default() -> Self {
        Self {
            start_time: None,
            last_intensity_a: 0,
            last_intensity_b: 0,
        }
    }
}

/// Maps audio analysis results to Coyote protocol commands
pub struct AudioMapper {
    config: MapperConfig,
    ramp_state: RampState,
    /// Last mapped Coyote frequency for channel A (for display/debugging)
    last_coyote_freq_a: u16,
    /// Last mapped Coyote frequency for channel B (for display/debugging)
    last_coyote_freq_b: u16,
}

impl AudioMapper {
    /// Create a new mapper with default configuration
    pub fn new() -> Self {
        let config = MapperConfig::default();
        let default_freq = config.default_coyote_freq;
        Self {
            config,
            ramp_state: RampState::default(),
            last_coyote_freq_a: default_freq,
            last_coyote_freq_b: default_freq,
        }
    }

    /// Create a new mapper with custom configuration
    pub fn with_config(config: MapperConfig) -> Self {
        let default_freq = config.default_coyote_freq;
        Self {
            config,
            ramp_state: RampState::default(),
            last_coyote_freq_a: default_freq,
            last_coyote_freq_b: default_freq,
        }
    }

    /// Update the mapper configuration
    pub fn set_config(&mut self, config: MapperConfig) {
        self.config = config;
    }

    /// Get the current mapper configuration
    pub fn config(&self) -> &MapperConfig {
        &self.config
    }

    /// Reset the ramp state (call when starting a new session)
    pub fn reset_ramp(&mut self) {
        self.ramp_state = RampState::default();
    }

    /// Get the last mapped Coyote frequency for channel A
    pub fn last_coyote_freq_a(&self) -> u16 {
        self.last_coyote_freq_a
    }

    /// Get the last mapped Coyote frequency for channel B
    pub fn last_coyote_freq_b(&self) -> u16 {
        self.last_coyote_freq_b
    }

    /// Map an analysis result to a Coyote command
    ///
    /// - Intensity is derived from amplitude (left -> channel A, right -> channel B)
    /// - Coyote frequency (X+Y) is derived from each channel's dominant audio frequency,
    ///   mapped through the configured frequency band (freq_band_min..freq_band_max)
    /// - Left audio frequency -> Channel A waveform frequency
    /// - Right audio frequency -> Channel B waveform frequency
    pub fn map(&mut self, analysis: &AnalysisResult, app_config: &Config) -> CoyoteCommand {
        // Calculate raw intensities from amplitude
        let raw_intensity_a =
            self.calculate_raw_intensity(analysis.left.amplitude, app_config.sensitivity);
        let raw_intensity_b =
            self.calculate_raw_intensity(analysis.right.amplitude, app_config.sensitivity);

        // Apply max intensity caps
        let capped_a = (raw_intensity_a as u32 * app_config.max_intensity_a as u32 / 2047) as u16;
        let capped_b = (raw_intensity_b as u32 * app_config.max_intensity_b as u32 / 2047) as u16;

        // Apply soft ramp-up
        let (ramped_a, ramped_b) = self.apply_ramp(capped_a, capped_b);

        // Calculate per-channel Coyote frequencies from each channel's dominant audio frequency
        let freq_a = self.map_frequency(
            analysis.left_frequency,
            app_config.freq_band_min,
            app_config.freq_band_max,
        );
        let freq_b = self.map_frequency(
            analysis.right_frequency,
            app_config.freq_band_min,
            app_config.freq_band_max,
        );
        self.last_coyote_freq_a = freq_a;
        self.last_coyote_freq_b = freq_b;

        // These should not fail since we clamp everything, but handle gracefully
        let intensity =
            Intensity::new(ramped_a.min(2047), ramped_b.min(2047)).unwrap_or_default();

        CoyoteCommand {
            intensity,
            freq_a,
            freq_b,
        }
    }

    /// Map the detected audio frequency to Coyote's frequency range
    ///
    /// Audio frequencies are clamped to [freq_band_min, freq_band_max], then mapped
    /// INVERSELY to output frequency:
    /// - High audio freq (band_max) → low Y → fast output (~100Hz)
    /// - Low audio freq (band_min) → high Y → slow output (~10Hz)
    fn map_frequency(
        &self,
        audio_freq: Option<f32>,
        band_min: f32,
        band_max: f32,
    ) -> u16 {
        // With X=1: coyote_freq = 1 + Y, so Y = coyote_freq - 1
        // coyote_freq=10 → Y=9 → period=10ms → 100Hz output (fast)
        // coyote_freq=100 → Y=99 → period=100ms → 10Hz output (slow)
        const COYOTE_FREQ_MIN: f32 = 10.0;  // 100Hz output (fast)
        const COYOTE_FREQ_MAX: f32 = 100.0; // 10Hz output (slow)

        match audio_freq {
            Some(freq) => {
                // Clamp to band edges
                let clamped = freq.clamp(band_min, band_max);
                // Normalize: 0 at band_min, 1 at band_max
                let normalized = (clamped - band_min) / (band_max - band_min);
                // INVERT: high audio freq (normalized=1) → low coyote_freq → fast output
                // low audio freq (normalized=0) → high coyote_freq → slow output
                let coyote_freq = COYOTE_FREQ_MAX - normalized * (COYOTE_FREQ_MAX - COYOTE_FREQ_MIN);
                (coyote_freq as u16).clamp(10, 100)
            }
            None => self.config.default_coyote_freq,
        }
    }

    /// Calculate raw intensity (0-2047) from amplitude (0.0-1.0)
    fn calculate_raw_intensity(&self, amplitude: f32, sensitivity: f32) -> u16 {
        // Apply sensitivity scaling (linear mapping)
        let scaled = (amplitude * sensitivity * 2.0).clamp(0.0, 1.0);

        // Scale to protocol range
        (scaled * Intensity::MAX as f32) as u16
    }

    /// Apply soft ramp-up to prevent sudden intensity jumps on initial connection.
    /// Only applies a brief startup ramp - user changes take effect immediately.
    fn apply_ramp(&mut self, target_a: u16, target_b: u16) -> (u16, u16) {
        let now = Instant::now();

        // Initialize ramp start if this is the first call
        if self.ramp_state.start_time.is_none() {
            self.ramp_state.start_time = Some(now);
        }

        let elapsed = now.duration_since(self.ramp_state.start_time.unwrap());

        // Brief startup ramp only - after ramp_duration, intensity is unrestricted
        if elapsed >= self.config.ramp_duration {
            // No ramp - immediate response to user settings
            self.ramp_state.last_intensity_a = target_a;
            self.ramp_state.last_intensity_b = target_b;
            return (target_a, target_b);
        }

        // During startup: limit max intensity based on time elapsed
        let ramp_factor = elapsed.as_secs_f32() / self.config.ramp_duration.as_secs_f32();
        let max_intensity = (Intensity::MAX as f32 * ramp_factor) as u16;

        let ramped_a = target_a.min(max_intensity);
        let ramped_b = target_b.min(max_intensity);

        self.ramp_state.last_intensity_a = ramped_a;
        self.ramp_state.last_intensity_b = ramped_b;

        (ramped_a, ramped_b)
    }
}

impl Default for AudioMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl AudioMapper {
    /// Skip the ramp-up period for testing (simulates enough time has passed)
    fn skip_ramp_for_test(&mut self) {
        self.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));
        self.ramp_state.last_intensity_a = 2047;
        self.ramp_state.last_intensity_b = 2047;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{ChannelResult, FrequencyBands};

    fn make_analysis(left_amp: f32, right_amp: f32) -> AnalysisResult {
        AnalysisResult {
            left: ChannelResult {
                amplitude: left_amp,
                frequency_bands: FrequencyBands::default(),
            },
            right: ChannelResult {
                amplitude: right_amp,
                frequency_bands: FrequencyBands::default(),
            },
            beat_detected: false,
            left_frequency: None,
            right_frequency: None,
            spectrum_left: [0.0; crate::audio::SPECTRUM_BARS],
            spectrum_right: [0.0; crate::audio::SPECTRUM_BARS],
        }
    }

    #[test]
    fn test_silence_maps_to_zero_intensity() {
        let mut mapper = AudioMapper::new();
        let analysis = make_analysis(0.0, 0.0);
        let config = Config::default();

        let cmd = mapper.map(&analysis, &config);

        assert_eq!(cmd.intensity.channel_a, 0);
        assert_eq!(cmd.intensity.channel_b, 0);
    }

    #[test]
    fn test_max_intensity_cap() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        let analysis = make_analysis(1.0, 1.0);
        let mut config = Config::default();
        config.max_intensity_a = 500;
        config.max_intensity_b = 750;

        let cmd = mapper.map(&analysis, &config);

        // Should be capped at max values
        assert!(cmd.intensity.channel_a <= 500);
        assert!(cmd.intensity.channel_b <= 750);
    }

    #[test]
    fn test_stereo_separation() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        let analysis = make_analysis(0.8, 0.2);
        let config = Config::default();

        let cmd = mapper.map(&analysis, &config);

        // Left (Channel A) should be higher than right (Channel B)
        assert!(cmd.intensity.channel_a > cmd.intensity.channel_b);
    }

    #[test]
    fn test_frequency_output_range() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        // Test that frequencies are in valid range (10-100)
        let mut analysis = make_analysis(0.5, 0.5);
        analysis.left_frequency = Some(500.0);
        analysis.right_frequency = Some(1000.0);
        let config = Config::default();

        let cmd = mapper.map(&analysis, &config);

        assert!(cmd.freq_a >= 10 && cmd.freq_a <= 100);
        assert!(cmd.freq_b >= 10 && cmd.freq_b <= 100);
    }

    #[test]
    fn test_ramp_up_initial() {
        let mut mapper = AudioMapper::new();
        let analysis = make_analysis(1.0, 1.0);
        let mut config = Config::default();
        config.max_intensity_a = 2047;
        config.max_intensity_b = 2047;
        config.sensitivity = 1.0;

        // First call should have limited intensity due to ramp
        let cmd = mapper.map(&analysis, &config);

        // Should be significantly below maximum due to ramp
        assert!(cmd.intensity.channel_a < 500);
        assert!(cmd.intensity.channel_b < 500);
    }

    #[test]
    fn test_reset_ramp() {
        let mut mapper = AudioMapper::new();
        // Simulate some activity
        mapper.ramp_state.start_time = Some(Instant::now());
        mapper.ramp_state.last_intensity_a = 1000;
        mapper.ramp_state.last_intensity_b = 1000;

        mapper.reset_ramp();

        assert!(mapper.ramp_state.start_time.is_none());
        assert_eq!(mapper.ramp_state.last_intensity_a, 0);
        assert_eq!(mapper.ramp_state.last_intensity_b, 0);
    }

    #[test]
    fn test_frequency_in_range_with_extreme_inputs() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        // Test with extreme analysis values
        let analysis = make_analysis(1.5, 1.5);
        let config = Config::default();

        let cmd = mapper.map(&analysis, &config);

        // Frequencies should be in valid range (10-100)
        assert!(cmd.freq_a >= 10 && cmd.freq_a <= 100);
        assert!(cmd.freq_b >= 10 && cmd.freq_b <= 100);
    }

    #[test]
    fn test_intensity_never_exceeds_max() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        // Extreme inputs
        let analysis = make_analysis(2.0, 2.0);
        let mut config = Config::default();
        config.max_intensity_a = 3000; // Over protocol max
        config.max_intensity_b = 3000;
        config.sensitivity = 2.0;

        let cmd = mapper.map(&analysis, &config);

        assert!(cmd.intensity.channel_a <= Intensity::MAX);
        assert!(cmd.intensity.channel_b <= Intensity::MAX);
    }

    #[test]
    fn test_sensitivity_scaling() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        let analysis = make_analysis(0.5, 0.5);

        let mut config_low = Config::default();
        config_low.sensitivity = 0.2;

        let mut config_high = Config::default();
        config_high.sensitivity = 1.0;

        let cmd_low = mapper.map(&analysis, &config_low);
        mapper.skip_ramp_for_test();
        let cmd_high = mapper.map(&analysis, &config_high);

        // Higher sensitivity should result in higher intensity
        assert!(cmd_high.intensity.channel_a > cmd_low.intensity.channel_a);
    }

    #[test]
    fn test_per_channel_frequency_mapping() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        // Create analysis with different frequencies for left and right
        let mut analysis = make_analysis(0.5, 0.5);
        analysis.left_frequency = Some(300.0);  // Low frequency
        analysis.right_frequency = Some(1500.0); // High frequency

        let mut config = Config::default();
        config.freq_band_min = 100.0;
        config.freq_band_max = 2000.0;

        let cmd = mapper.map(&analysis, &config);

        // Mapping is INVERTED: high audio freq → low coyote_freq → fast output
        // 300 Hz (low audio) → high coyote_freq → slow output
        // 1500 Hz (high audio) → low coyote_freq → fast output

        // Channel A (left, low audio 300Hz) should have HIGHER Coyote freq than Channel B (right, high audio 1500Hz)
        assert!(
            cmd.freq_a > cmd.freq_b,
            "Left freq 300 Hz should map to HIGHER Coyote freq (slower output) than right 1500 Hz, but got A={} B={}",
            cmd.freq_a, cmd.freq_b
        );
    }

    #[test]
    fn test_frequency_fallback_to_default() {
        let mut mapper = AudioMapper::new();
        mapper.skip_ramp_for_test();

        // Create analysis with no detected frequencies
        let analysis = make_analysis(0.5, 0.5);
        let config = Config::default();

        let _cmd = mapper.map(&analysis, &config);

        // Both channels should use the default Coyote frequency (50 = middle of range)
        assert_eq!(mapper.last_coyote_freq_a(), 50);
        assert_eq!(mapper.last_coyote_freq_b(), 50);
    }
}
