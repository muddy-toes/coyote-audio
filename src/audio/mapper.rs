//! Maps audio analysis results to Coyote V2 protocol commands.
//!
//! Amplitude always controls intensity. The detected dominant frequency is mapped
//! to the Coyote's frequency output (X+Y) based on a user-configured frequency band.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ble::protocol::{Intensity, Waveform, WaveformParams};
use crate::config::Config;

use super::AnalysisResult;

/// Mapping curve for intensity scaling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MappingCurve {
    /// Linear mapping: output = input
    #[default]
    Linear,
    /// Exponential: more responsive at low levels, compressed at high
    /// output = input^2
    Exponential,
    /// Logarithmic: more responsive at high levels, compressed at low
    /// output = log(1 + input * (e - 1))
    Logarithmic,
    /// S-Curve: smooth transition, gentle at extremes, steeper in middle
    /// Uses a smoothstep function: 3x^2 - 2x^3
    SCurve,
}

impl MappingCurve {
    /// Apply the curve to a normalized value (0.0-1.0)
    pub fn apply(&self, value: f32) -> f32 {
        let clamped = value.clamp(0.0, 1.0);
        match self {
            MappingCurve::Linear => clamped,
            MappingCurve::Exponential => clamped * clamped,
            MappingCurve::Logarithmic => {
                // log(1 + x * (e - 1)) for x in [0,1] maps to [0,1]
                let e_minus_1 = std::f32::consts::E - 1.0;
                (1.0 + clamped * e_minus_1).ln()
            }
            MappingCurve::SCurve => {
                // Smoothstep: 3x^2 - 2x^3
                // Gentle at 0 and 1, steeper in the middle
                clamped * clamped * (3.0 - 2.0 * clamped)
            }
        }
    }
}

/// Configuration for the audio-to-signal mapper
#[derive(Debug, Clone)]
pub struct MapperConfig {
    /// Intensity mapping curve
    pub intensity_curve: MappingCurve,
    /// Fixed X (pulse burst duration) value (0-31 ms)
    pub x_value: u8,
    /// Fixed Z (pulse width) value (0-31, units of 5us)
    pub z_value: u8,
    /// Minimum pulse width Z value (safety)
    pub min_pulse_width: u8,
    /// Default frequency when audio frequency is outside configured band (Coyote freq 10-100)
    pub default_coyote_freq: u16,
    /// Soft ramp-up duration
    pub ramp_duration: Duration,
}

impl Default for MapperConfig {
    fn default() -> Self {
        Self {
            intensity_curve: MappingCurve::Linear,
            x_value: 10,
            z_value: 10,
            min_pulse_width: 5,
            default_coyote_freq: 55, // Middle of Coyote's 10-100 range
            ramp_duration: Duration::from_millis(500),
        }
    }
}

/// Command ready to send to the Coyote device
#[derive(Debug, Clone)]
pub struct CoyoteCommand {
    pub intensity: Intensity,
    pub waveform_a: Waveform,
    pub waveform_b: Waveform,
}

impl Default for CoyoteCommand {
    fn default() -> Self {
        Self {
            intensity: Intensity::default(),
            waveform_a: Waveform::default(),
            waveform_b: Waveform::default(),
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
        Self {
            config: MapperConfig::default(),
            ramp_state: RampState::default(),
            last_coyote_freq_a: 55,
            last_coyote_freq_b: 55,
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

    /// Set the intensity mapping curve
    pub fn set_intensity_curve(&mut self, curve: MappingCurve) {
        self.config.intensity_curve = curve;
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
        let coyote_freq_a = self.map_frequency(
            analysis.left_frequency,
            app_config.freq_band_min,
            app_config.freq_band_max,
        );
        let coyote_freq_b = self.map_frequency(
            analysis.right_frequency,
            app_config.freq_band_min,
            app_config.freq_band_max,
        );
        self.last_coyote_freq_a = coyote_freq_a;
        self.last_coyote_freq_b = coyote_freq_b;

        // Calculate waveform parameters for each channel
        let waveform_a = self.calculate_waveform(coyote_freq_a);
        let waveform_b = self.calculate_waveform(coyote_freq_b);

        // These should not fail since we clamp everything, but handle gracefully
        let intensity =
            Intensity::new(ramped_a.min(2047), ramped_b.min(2047)).unwrap_or_default();

        CoyoteCommand {
            intensity,
            waveform_a,
            waveform_b,
        }
    }

    /// Map the detected audio frequency to Coyote's frequency range (10-100)
    ///
    /// If the audio frequency is within [freq_band_min, freq_band_max], map it linearly
    /// to Coyote's [10, 100] range. Otherwise, use the default frequency.
    fn map_frequency(
        &self,
        audio_freq: Option<f32>,
        band_min: f32,
        band_max: f32,
    ) -> u16 {
        const COYOTE_FREQ_MIN: f32 = 10.0;
        const COYOTE_FREQ_MAX: f32 = 100.0;

        match audio_freq {
            Some(freq) if freq >= band_min && freq <= band_max => {
                // Linear mapping from [band_min, band_max] to [10, 100]
                let normalized = (freq - band_min) / (band_max - band_min);
                let coyote_freq = COYOTE_FREQ_MIN + normalized * (COYOTE_FREQ_MAX - COYOTE_FREQ_MIN);
                (coyote_freq as u16).clamp(10, 100)
            }
            _ => self.config.default_coyote_freq,
        }
    }

    /// Calculate raw intensity (0-2047) from amplitude (0.0-1.0)
    fn calculate_raw_intensity(&self, amplitude: f32, sensitivity: f32) -> u16 {
        // Apply sensitivity scaling
        let scaled = (amplitude * sensitivity * 2.0).clamp(0.0, 1.0);

        // Apply the mapping curve
        let curved = self.config.intensity_curve.apply(scaled);

        // Scale to protocol range
        (curved * Intensity::MAX as f32) as u16
    }

    /// Apply soft ramp-up to prevent sudden intensity jumps
    fn apply_ramp(&mut self, target_a: u16, target_b: u16) -> (u16, u16) {
        let now = Instant::now();

        // Initialize ramp start if this is the first call
        if self.ramp_state.start_time.is_none() {
            self.ramp_state.start_time = Some(now);
        }

        let elapsed = now.duration_since(self.ramp_state.start_time.unwrap());

        // Calculate ramp factor (0.0 to 1.0)
        let ramp_factor = if elapsed >= self.config.ramp_duration {
            1.0
        } else {
            elapsed.as_secs_f32() / self.config.ramp_duration.as_secs_f32()
        };

        // Apply ramp to limit how fast intensity can increase
        let max_increase_a = (Intensity::MAX as f32 * ramp_factor) as u16;
        let max_increase_b = (Intensity::MAX as f32 * ramp_factor) as u16;

        // Calculate maximum allowed intensity based on ramp
        let allowed_a = max_increase_a.min(target_a);
        let allowed_b = max_increase_b.min(target_b);

        // Also limit the rate of increase from the last value
        // Allow decreasing instantly but increasing only gradually
        let max_step = (Intensity::MAX as f32 * 0.1) as u16; // 10% per step max increase

        let ramped_a = if allowed_a > self.ramp_state.last_intensity_a {
            (self.ramp_state.last_intensity_a + max_step).min(allowed_a)
        } else {
            allowed_a
        };

        let ramped_b = if allowed_b > self.ramp_state.last_intensity_b {
            (self.ramp_state.last_intensity_b + max_step).min(allowed_b)
        } else {
            allowed_b
        };

        // Update last values
        self.ramp_state.last_intensity_a = ramped_a;
        self.ramp_state.last_intensity_b = ramped_b;

        (ramped_a, ramped_b)
    }

    /// Calculate waveform parameters from Coyote frequency
    ///
    /// Coyote frequency = X + Y, where X is pulse burst duration and Y is interval.
    /// We use a fixed X and calculate Y = freq - X.
    fn calculate_waveform(&self, coyote_freq: u16) -> Waveform {
        let x = self.config.x_value.min(WaveformParams::X_MAX);

        // Y = freq - X, clamped to valid range
        let y_val = (coyote_freq as i32 - x as i32).max(0);
        let y = (y_val as u16).min(WaveformParams::Y_MAX);

        // Z is fixed, but respect minimum pulse width safety
        let z = self
            .config
            .z_value
            .max(self.config.min_pulse_width)
            .min(WaveformParams::Z_MAX);

        // Create the waveform (this should not fail with our clamping)
        let params = WaveformParams::new(x, y, z).unwrap_or_default();
        Waveform::new(params)
    }
}

impl Default for AudioMapper {
    fn default() -> Self {
        Self::new()
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
        }
    }

    #[test]
    fn test_mapping_curve_linear() {
        let curve = MappingCurve::Linear;
        assert_eq!(curve.apply(0.0), 0.0);
        assert_eq!(curve.apply(0.5), 0.5);
        assert_eq!(curve.apply(1.0), 1.0);
    }

    #[test]
    fn test_mapping_curve_exponential() {
        let curve = MappingCurve::Exponential;
        assert_eq!(curve.apply(0.0), 0.0);
        assert_eq!(curve.apply(0.5), 0.25); // 0.5^2
        assert_eq!(curve.apply(1.0), 1.0);
    }

    #[test]
    fn test_mapping_curve_logarithmic() {
        let curve = MappingCurve::Logarithmic;
        assert_eq!(curve.apply(0.0), 0.0);
        assert!((curve.apply(1.0) - 1.0).abs() < 0.001);
        // Logarithmic should be higher than linear at midpoint
        assert!(curve.apply(0.5) > 0.5);
    }

    #[test]
    fn test_mapping_curve_scurve() {
        let curve = MappingCurve::SCurve;
        assert_eq!(curve.apply(0.0), 0.0);
        assert_eq!(curve.apply(1.0), 1.0);
        // S-curve at 0.5 should equal 0.5 (inflection point)
        assert!((curve.apply(0.5) - 0.5).abs() < 0.001);
        // S-curve should be below linear at 0.25 (gentle start)
        assert!(curve.apply(0.25) < 0.25);
        // S-curve should be above linear at 0.75 (steep middle already passed)
        assert!(curve.apply(0.75) > 0.75);
    }

    #[test]
    fn test_mapping_curve_clamps_input() {
        let curve = MappingCurve::Linear;
        assert_eq!(curve.apply(-0.5), 0.0);
        assert_eq!(curve.apply(1.5), 1.0);
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
        // Skip ramp by calling multiple times
        mapper.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));
        mapper.ramp_state.last_intensity_a = 2047;
        mapper.ramp_state.last_intensity_b = 2047;

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
        // Skip ramp
        mapper.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));
        mapper.ramp_state.last_intensity_a = 2047;
        mapper.ramp_state.last_intensity_b = 2047;

        let analysis = make_analysis(0.8, 0.2);
        let config = Config::default();

        let cmd = mapper.map(&analysis, &config);

        // Left (Channel A) should be higher than right (Channel B)
        assert!(cmd.intensity.channel_a > cmd.intensity.channel_b);
    }

    #[test]
    fn test_x_value_fixed() {
        let config = MapperConfig {
            x_value: 15,
            ..Default::default()
        };
        let mapper = AudioMapper::with_config(config);
        let waveform = mapper.calculate_waveform(200);

        assert_eq!(waveform.params.x, 15);
    }

    #[test]
    fn test_z_value_fixed() {
        let config = MapperConfig {
            z_value: 20,
            min_pulse_width: 5,
            ..Default::default()
        };
        let mapper = AudioMapper::with_config(config);
        let waveform = mapper.calculate_waveform(200);

        assert_eq!(waveform.params.z, 20);
    }

    #[test]
    fn test_min_pulse_width_safety() {
        let config = MapperConfig {
            z_value: 2,
            min_pulse_width: 5,
            ..Default::default()
        };
        let mapper = AudioMapper::with_config(config);
        let waveform = mapper.calculate_waveform(200);

        // Should be clamped to minimum
        assert_eq!(waveform.params.z, 5);
    }

    #[test]
    fn test_y_from_frequency() {
        let config = MapperConfig {
            x_value: 10,
            ..Default::default()
        };
        let mapper = AudioMapper::with_config(config);

        // Y = coyote_freq - X = 200 - 10 = 190
        let waveform = mapper.calculate_waveform(200);
        assert_eq!(waveform.params.y, 190);

        // Y = coyote_freq - X = 100 - 10 = 90
        let waveform2 = mapper.calculate_waveform(100);
        assert_eq!(waveform2.params.y, 90);
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
    fn test_waveform_params_in_range() {
        let mut mapper = AudioMapper::with_config(MapperConfig {
            x_value: 31,
            z_value: 31,
            min_pulse_width: 0,
            ..Default::default()
        });

        // Test with extreme analysis values
        let analysis = make_analysis(1.5, 1.5);
        let config = Config::default();

        let cmd = mapper.map(&analysis, &config);

        // All waveform params should be within valid ranges
        assert!(cmd.waveform_a.params.x <= WaveformParams::X_MAX);
        assert!(cmd.waveform_a.params.y <= WaveformParams::Y_MAX);
        assert!(cmd.waveform_a.params.z <= WaveformParams::Z_MAX);
    }

    #[test]
    fn test_intensity_never_exceeds_max() {
        let mut mapper = AudioMapper::new();
        // Skip ramp
        mapper.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));
        mapper.ramp_state.last_intensity_a = 2047;
        mapper.ramp_state.last_intensity_b = 2047;

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
        // Skip ramp
        mapper.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));
        mapper.ramp_state.last_intensity_a = 2047;
        mapper.ramp_state.last_intensity_b = 2047;

        let analysis = make_analysis(0.5, 0.5);

        let mut config_low = Config::default();
        config_low.sensitivity = 0.2;

        let mut config_high = Config::default();
        config_high.sensitivity = 1.0;

        let cmd_low = mapper.map(&analysis, &config_low);
        mapper.ramp_state.last_intensity_a = 2047;
        mapper.ramp_state.last_intensity_b = 2047;
        let cmd_high = mapper.map(&analysis, &config_high);

        // Higher sensitivity should result in higher intensity
        assert!(cmd_high.intensity.channel_a > cmd_low.intensity.channel_a);
    }

    #[test]
    fn test_per_channel_frequency_mapping() {
        let mut mapper = AudioMapper::new();
        // Skip ramp
        mapper.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));
        mapper.ramp_state.last_intensity_a = 2047;
        mapper.ramp_state.last_intensity_b = 2047;

        // Create analysis with different frequencies for left and right
        let mut analysis = make_analysis(0.5, 0.5);
        analysis.left_frequency = Some(300.0);  // Low frequency
        analysis.right_frequency = Some(1500.0); // High frequency

        let mut config = Config::default();
        config.freq_band_min = 100.0;
        config.freq_band_max = 2000.0;

        let cmd = mapper.map(&analysis, &config);

        // Verify the mapper tracks different frequencies for each channel
        let freq_a = mapper.last_coyote_freq_a();
        let freq_b = mapper.last_coyote_freq_b();

        // 300 Hz is at (300-100)/(2000-100) = 200/1900 ~= 10.5% of the range
        // Maps to 10 + 0.105 * 90 ~= 19
        // 1500 Hz is at (1500-100)/(2000-100) = 1400/1900 ~= 73.7% of the range
        // Maps to 10 + 0.737 * 90 ~= 76

        // Channel A (left) should have lower Coyote freq than Channel B (right)
        assert!(
            freq_a < freq_b,
            "Left freq {} Hz should map to lower Coyote freq than right, but got A={} B={}",
            300.0, freq_a, freq_b
        );

        // Verify waveforms are different
        assert!(
            cmd.waveform_a.params.y < cmd.waveform_b.params.y,
            "Waveform A Y should be less than B, got A.y={} B.y={}",
            cmd.waveform_a.params.y, cmd.waveform_b.params.y
        );
    }

    #[test]
    fn test_frequency_fallback_to_default() {
        let mut mapper = AudioMapper::new();
        mapper.ramp_state.start_time = Some(Instant::now() - Duration::from_secs(10));

        // Create analysis with no detected frequencies
        let analysis = make_analysis(0.5, 0.5);
        let config = Config::default();

        let _cmd = mapper.map(&analysis, &config);

        // Both channels should use the default Coyote frequency (55)
        assert_eq!(mapper.last_coyote_freq_a(), 55);
        assert_eq!(mapper.last_coyote_freq_b(), 55);
    }
}
