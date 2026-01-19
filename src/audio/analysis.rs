//! Audio analysis: computes amplitude, beat detection, and frequency bands.

use std::collections::VecDeque;

use rustfft::{num_complex::Complex, FftPlanner};

use super::AudioBuffer;

/// Analysis mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnalysisMode {
    /// Simple amplitude tracking
    #[default]
    Amplitude,
    /// Beat detection using energy spikes
    BeatDetection,
    /// Frequency band analysis (bass/mid/treble)
    FrequencyBands,
}

/// Beat detection configuration
#[derive(Debug, Clone)]
pub struct BeatDetectionConfig {
    /// Number of energy history frames to keep
    pub history_size: usize,
    /// Multiplier above average energy to detect beat
    pub threshold_multiplier: f32,
    /// Minimum time between beats (ms)
    pub min_beat_interval_ms: u64,
}

impl Default for BeatDetectionConfig {
    fn default() -> Self {
        Self {
            history_size: 43, // ~1 second at 30fps
            threshold_multiplier: 1.5,
            min_beat_interval_ms: 100,
        }
    }
}

/// Frequency band energy levels
#[derive(Debug, Clone, Default)]
pub struct FrequencyBands {
    /// Bass energy (20-250 Hz), normalized 0.0-1.0
    pub bass: f32,
    /// Mid energy (250-4000 Hz), normalized 0.0-1.0
    pub mid: f32,
    /// Treble energy (4000-20000 Hz), normalized 0.0-1.0
    pub treble: f32,
}

/// Number of spectrum analyzer bars to display
pub const SPECTRUM_BARS: usize = 32;

/// Result of audio analysis
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Left channel data
    pub left: ChannelResult,
    /// Right channel data
    pub right: ChannelResult,
    /// Whether a beat was detected this frame
    pub beat_detected: bool,
    /// Dominant frequency detected in left channel (Hz), or None if below threshold
    pub left_frequency: Option<f32>,
    /// Dominant frequency detected in right channel (Hz), or None if below threshold
    pub right_frequency: Option<f32>,
    /// Spectrum analyzer data for left channel, normalized 0.0-1.0 per bar
    /// Covers 20Hz-20kHz in logarithmic frequency bands
    pub spectrum_left: [f32; SPECTRUM_BARS],
    /// Spectrum analyzer data for right channel, normalized 0.0-1.0 per bar
    pub spectrum_right: [f32; SPECTRUM_BARS],
}

/// Per-channel analysis result
#[derive(Debug, Clone, Default)]
pub struct ChannelResult {
    /// RMS amplitude (0.0-1.0)
    pub amplitude: f32,
    /// Frequency band energies
    pub frequency_bands: FrequencyBands,
}

/// Audio analyzer that processes audio buffers
pub struct AudioAnalyzer {
    mode: AnalysisMode,
    sample_rate: u32,
    /// FFT planner for frequency analysis
    fft_planner: FftPlanner<f32>,
    /// Scratch buffer for FFT input/output
    fft_buffer: Vec<Complex<f32>>,
    /// Minimum magnitude threshold for considering a frequency peak valid
    magnitude_threshold: f32,
    /// Energy history for beat detection
    energy_history: VecDeque<f32>,
    /// Beat detection config
    beat_config: BeatDetectionConfig,
    /// Smoothed frequency for left channel (EMA)
    smoothed_left_freq: Option<f32>,
    /// Smoothed frequency for right channel (EMA)
    smoothed_right_freq: Option<f32>,
    /// EMA smoothing factor (0.0-1.0, higher = more responsive)
    freq_smoothing_alpha: f32,
}

impl AudioAnalyzer {
    /// Create a new audio analyzer with the specified mode and sample rate
    pub fn new(mode: AnalysisMode, sample_rate: u32) -> Self {
        Self {
            mode,
            sample_rate,
            fft_planner: FftPlanner::new(),
            fft_buffer: Vec::new(),
            magnitude_threshold: 0.01,
            energy_history: VecDeque::new(),
            beat_config: BeatDetectionConfig::default(),
            smoothed_left_freq: None,
            smoothed_right_freq: None,
            freq_smoothing_alpha: 0.7, // Higher = more responsive to new readings
        }
    }

    /// Get the current analysis mode
    pub fn mode(&self) -> AnalysisMode {
        self.mode
    }

    /// Set the analysis mode
    pub fn set_mode(&mut self, mode: AnalysisMode) {
        self.mode = mode;
        // Clear history when switching modes
        self.energy_history.clear();
        // Reset frequency smoothing state
        self.smoothed_left_freq = None;
        self.smoothed_right_freq = None;
    }

    /// Update the sample rate (call when audio format changes)
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    /// Apply EMA smoothing to a frequency value
    fn apply_smoothing(&self, new_freq: Option<f32>, prev_smoothed: &mut Option<f32>) -> Option<f32> {
        match (new_freq, *prev_smoothed) {
            (Some(new), Some(prev)) => {
                let smoothed = self.freq_smoothing_alpha * new + (1.0 - self.freq_smoothing_alpha) * prev;
                *prev_smoothed = Some(smoothed);
                Some(smoothed)
            }
            (Some(new), None) => {
                *prev_smoothed = Some(new);
                Some(new)
            }
            (None, _) => {
                *prev_smoothed = None;
                None
            }
        }
    }

    /// Analyze an audio buffer
    pub fn analyze(&mut self, buffer: &AudioBuffer) -> AnalysisResult {
        let (left_samples, right_samples) = self.deinterleave_stereo(buffer);

        let left_rms = calculate_rms(&left_samples);
        let right_rms = calculate_rms(&right_samples);

        // Find dominant frequency for each channel independently
        let raw_left_freq = self.find_dominant_frequency(&left_samples);
        let raw_right_freq = self.find_dominant_frequency(&right_samples);

        // Apply EMA smoothing to reduce bin-hopping jitter
        let mut smoothed_left = self.smoothed_left_freq;
        let mut smoothed_right = self.smoothed_right_freq;
        let left_frequency = self.apply_smoothing(raw_left_freq, &mut smoothed_left);
        let right_frequency = self.apply_smoothing(raw_right_freq, &mut smoothed_right);
        self.smoothed_left_freq = smoothed_left;
        self.smoothed_right_freq = smoothed_right;

        // Calculate frequency bands
        let left_bands = self.calculate_frequency_bands(&left_samples);
        let right_bands = self.calculate_frequency_bands(&right_samples);

        // Beat detection
        let beat_detected = self.detect_beat(left_rms, right_rms);

        // Calculate spectrum analyzer data for each channel
        let spectrum_left = self.calculate_spectrum(&left_samples);
        let spectrum_right = self.calculate_spectrum(&right_samples);

        AnalysisResult {
            left: ChannelResult {
                amplitude: left_rms,
                frequency_bands: left_bands,
            },
            right: ChannelResult {
                amplitude: right_rms,
                frequency_bands: right_bands,
            },
            beat_detected,
            left_frequency,
            right_frequency,
            spectrum_left,
            spectrum_right,
        }
    }

    /// Detect beats based on energy spikes
    fn detect_beat(&mut self, left_rms: f32, right_rms: f32) -> bool {
        let energy = (left_rms + right_rms) / 2.0;

        // Add to history
        self.energy_history.push_back(energy);
        if self.energy_history.len() > self.beat_config.history_size {
            self.energy_history.pop_front();
        }

        // Need enough history
        if self.energy_history.len() < 10 {
            return false;
        }

        // Calculate average energy
        let avg: f32 = self.energy_history.iter().sum::<f32>() / self.energy_history.len() as f32;

        // Beat if current energy exceeds threshold above average
        energy > avg * self.beat_config.threshold_multiplier
    }

    /// Calculate frequency bands from samples using FFT
    fn calculate_frequency_bands(&mut self, samples: &[f32]) -> FrequencyBands {
        if samples.is_empty() {
            return FrequencyBands::default();
        }

        // Prepare FFT
        let fft_size = samples.len().next_power_of_two();
        self.fft_buffer.clear();
        self.fft_buffer.reserve(fft_size);

        for (i, &sample) in samples.iter().enumerate() {
            let window = 0.5
                * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / (samples.len() - 1) as f32).cos());
            self.fft_buffer.push(Complex::new(sample * window, 0.0));
        }
        self.fft_buffer.resize(fft_size, Complex::new(0.0, 0.0));

        let fft = self.fft_planner.plan_fft_forward(fft_size);
        fft.process(&mut self.fft_buffer);

        let freq_resolution = self.sample_rate as f32 / fft_size as f32;
        let nyquist_bin = fft_size / 2;

        // Calculate band energies
        let mut bass_energy = 0.0f32;
        let mut mid_energy = 0.0f32;
        let mut treble_energy = 0.0f32;

        for (i, c) in self.fft_buffer[1..nyquist_bin].iter().enumerate() {
            let freq = (i + 1) as f32 * freq_resolution;
            let magnitude = c.norm();

            if freq >= 20.0 && freq < 250.0 {
                bass_energy += magnitude;
            } else if freq >= 250.0 && freq < 4000.0 {
                mid_energy += magnitude;
            } else if freq >= 4000.0 && freq <= 20000.0 {
                treble_energy += magnitude;
            }
        }

        // Normalize (rough approximation)
        let max_energy = bass_energy.max(mid_energy).max(treble_energy).max(0.001);
        FrequencyBands {
            bass: (bass_energy / max_energy).clamp(0.0, 1.0),
            mid: (mid_energy / max_energy).clamp(0.0, 1.0),
            treble: (treble_energy / max_energy).clamp(0.0, 1.0),
        }
    }

    /// Find the dominant frequency in the audio using FFT
    fn find_dominant_frequency(&mut self, samples: &[f32]) -> Option<f32> {
        if samples.is_empty() {
            return None;
        }

        // Use at least 8192-point FFT for good frequency resolution (~5.9 Hz at 48kHz)
        // This allows distinguishing frequencies that are close together
        let min_fft_size = 8192;
        let fft_size = samples.len().next_power_of_two().max(min_fft_size);
        self.fft_buffer.clear();
        self.fft_buffer.reserve(fft_size);

        for (i, &sample) in samples.iter().enumerate() {
            // Hann window to reduce spectral leakage
            let window = 0.5
                * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / (samples.len() - 1) as f32).cos());
            self.fft_buffer.push(Complex::new(sample * window, 0.0));
        }

        // Zero-pad to power of two
        self.fft_buffer.resize(fft_size, Complex::new(0.0, 0.0));

        // Perform FFT
        let fft = self.fft_planner.plan_fft_forward(fft_size);
        fft.process(&mut self.fft_buffer);

        // Calculate frequency resolution
        let freq_resolution = self.sample_rate as f32 / fft_size as f32;

        // Only consider positive frequencies (first half of FFT output)
        // and skip DC component (bin 0)
        let nyquist_bin = fft_size / 2;

        // Find bin with maximum magnitude
        let mut max_magnitude: f32 = 0.0;
        let mut max_bin: usize = 0;

        for (i, c) in self.fft_buffer[1..nyquist_bin].iter().enumerate() {
            let magnitude = c.norm();
            if magnitude > max_magnitude {
                max_magnitude = magnitude;
                max_bin = i + 1; // +1 because we skipped bin 0
            }
        }

        // Check if the magnitude exceeds our threshold
        if max_magnitude < self.magnitude_threshold {
            return None;
        }

        // Convert bin index to frequency
        let frequency = max_bin as f32 * freq_resolution;

        // Only return frequencies in audible range (20 Hz - 20 kHz)
        if frequency >= 20.0 && frequency <= 20000.0 {
            Some(frequency)
        } else {
            None
        }
    }

    /// Calculate spectrum analyzer data for a single channel
    /// Returns SPECTRUM_BARS values (0.0-1.0) covering 20Hz-20kHz in log-spaced bands
    fn calculate_spectrum(&mut self, samples: &[f32]) -> [f32; SPECTRUM_BARS] {
        let mut spectrum = [0.0f32; SPECTRUM_BARS];

        if samples.is_empty() {
            return spectrum;
        }

        // Use at least 4096-point FFT for decent resolution
        let min_fft_size = 4096;
        let fft_size = samples.len().next_power_of_two().max(min_fft_size);
        self.fft_buffer.clear();
        self.fft_buffer.reserve(fft_size);

        // Apply Hann window
        for (i, &sample) in samples.iter().enumerate() {
            let window = 0.5
                * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / (samples.len() - 1).max(1) as f32)
                        .cos());
            self.fft_buffer.push(Complex::new(sample * window, 0.0));
        }
        self.fft_buffer.resize(fft_size, Complex::new(0.0, 0.0));

        // Perform FFT
        let fft = self.fft_planner.plan_fft_forward(fft_size);
        fft.process(&mut self.fft_buffer);

        let freq_resolution = self.sample_rate as f32 / fft_size as f32;
        let nyquist_bin = fft_size / 2;

        // Define logarithmically-spaced frequency bands from 20Hz to 20kHz
        let min_freq = 20.0f32;
        let max_freq = 20000.0f32;
        let log_min = min_freq.ln();
        let log_max = max_freq.ln();

        // For each spectrum bar, sum magnitudes in its frequency range
        let mut max_magnitude = 0.0f32;
        for bar in 0..SPECTRUM_BARS {
            let t0 = bar as f32 / SPECTRUM_BARS as f32;
            let t1 = (bar + 1) as f32 / SPECTRUM_BARS as f32;
            let freq_low = (log_min + t0 * (log_max - log_min)).exp();
            let freq_high = (log_min + t1 * (log_max - log_min)).exp();

            let bin_low = ((freq_low / freq_resolution) as usize).max(1);
            let bin_high = ((freq_high / freq_resolution) as usize).min(nyquist_bin);

            let mut sum = 0.0f32;
            let mut count = 0;
            for bin in bin_low..bin_high {
                if bin < self.fft_buffer.len() {
                    sum += self.fft_buffer[bin].norm();
                    count += 1;
                }
            }

            spectrum[bar] = if count > 0 { sum / count as f32 } else { 0.0 };
            max_magnitude = max_magnitude.max(spectrum[bar]);
        }

        // Normalize to 0.0-1.0
        if max_magnitude > 0.0 {
            for bar in spectrum.iter_mut() {
                *bar = (*bar / max_magnitude).clamp(0.0, 1.0);
            }
        }

        spectrum
    }

    /// Separate interleaved stereo samples into left and right channels
    fn deinterleave_stereo(&self, buffer: &AudioBuffer) -> (Vec<f32>, Vec<f32>) {
        if buffer.channels != 2 {
            // Mono or unexpected format: duplicate to both channels
            return (buffer.samples.clone(), buffer.samples.clone());
        }

        let frame_count = buffer.samples.len() / 2;
        let mut left = Vec::with_capacity(frame_count);
        let mut right = Vec::with_capacity(frame_count);

        for chunk in buffer.samples.chunks(2) {
            if chunk.len() == 2 {
                left.push(chunk[0]);
                right.push(chunk[1]);
            }
        }

        (left, right)
    }
}

/// Calculate RMS (Root Mean Square) of samples, normalized to 0.0-1.0
fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_squares / samples.len() as f32).sqrt();

    // Normalize: f32 audio typically ranges from -1.0 to 1.0
    // RMS of full-scale sine wave is ~0.707, so we scale accordingly
    // to make 0.707 map to approximately 1.0 for typical audio
    (rms * 1.414).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_buffer(samples: Vec<f32>) -> AudioBuffer {
        AudioBuffer {
            samples,
            channels: 2,
            sample_rate: 48000,
        }
    }

    #[test]
    fn test_amplitude_silence() {
        let mut analyzer = AudioAnalyzer::new(AnalysisMode::Amplitude, 48000);
        let buffer = create_test_buffer(vec![0.0; 1024]);
        let result = analyzer.analyze(&buffer);

        assert_eq!(result.left.amplitude, 0.0);
        assert_eq!(result.right.amplitude, 0.0);
        assert!(result.left_frequency.is_none());
        assert!(result.right_frequency.is_none());
    }

    #[test]
    fn test_amplitude_full_scale() {
        let mut analyzer = AudioAnalyzer::new(AnalysisMode::Amplitude, 48000);
        // Create a full-scale square wave (alternating +1, -1)
        let samples: Vec<f32> = (0..1024)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        // Interleave as stereo
        let stereo: Vec<f32> = samples.iter().flat_map(|&s| [s, s]).collect();
        let buffer = create_test_buffer(stereo);
        let result = analyzer.analyze(&buffer);

        // Full scale square wave should give high amplitude
        assert!(result.left.amplitude > 0.9);
        assert!(result.right.amplitude > 0.9);
    }

    #[test]
    fn test_amplitude_stereo_separation() {
        let mut analyzer = AudioAnalyzer::new(AnalysisMode::Amplitude, 48000);
        // Left channel loud, right channel quiet
        let stereo: Vec<f32> = (0..512).flat_map(|_| [0.8, 0.1]).collect();
        let buffer = create_test_buffer(stereo);
        let result = analyzer.analyze(&buffer);

        assert!(result.left.amplitude > result.right.amplitude);
    }

    #[test]
    fn test_dominant_frequency_440hz() {
        let mut analyzer = AudioAnalyzer::new(AnalysisMode::FrequencyBands, 48000);

        // Generate a 440 Hz sine wave (A4 note)
        let freq = 440.0;
        let sample_rate = 48000.0;
        let samples: Vec<f32> = (0..2048)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8
            })
            .collect();
        let stereo: Vec<f32> = samples.iter().flat_map(|&s| [s, s]).collect();
        let buffer = AudioBuffer {
            samples: stereo,
            channels: 2,
            sample_rate: 48000,
        };

        let result = analyzer.analyze(&buffer);

        // Should detect a frequency close to 440 Hz on both channels (same content)
        assert!(result.left_frequency.is_some());
        assert!(result.right_frequency.is_some());
        let detected_left = result.left_frequency.unwrap();
        let detected_right = result.right_frequency.unwrap();
        // Allow some tolerance due to FFT bin resolution
        assert!(
            (detected_left - 440.0).abs() < 50.0,
            "Expected ~440 Hz on left, got {} Hz",
            detected_left
        );
        assert!(
            (detected_right - 440.0).abs() < 50.0,
            "Expected ~440 Hz on right, got {} Hz",
            detected_right
        );
    }

    #[test]
    fn test_dominant_frequency_bass() {
        let mut analyzer = AudioAnalyzer::new(AnalysisMode::FrequencyBands, 48000);

        // Generate a 100 Hz sine wave (bass range)
        let freq = 100.0;
        let sample_rate = 48000.0;
        let samples: Vec<f32> = (0..4096)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8
            })
            .collect();
        let stereo: Vec<f32> = samples.iter().flat_map(|&s| [s, s]).collect();
        let buffer = AudioBuffer {
            samples: stereo,
            channels: 2,
            sample_rate: 48000,
        };

        let result = analyzer.analyze(&buffer);

        assert!(result.left_frequency.is_some());
        let detected = result.left_frequency.unwrap();
        assert!(
            (detected - 100.0).abs() < 30.0,
            "Expected ~100 Hz, got {} Hz",
            detected
        );
    }

    #[test]
    fn test_stereo_frequency_separation() {
        let mut analyzer = AudioAnalyzer::new(AnalysisMode::FrequencyBands, 48000);

        // Generate different frequencies for left (200 Hz) and right (800 Hz) channels
        let left_freq = 200.0;
        let right_freq = 800.0;
        let sample_rate = 48000.0;
        let num_samples = 4096;

        let mut stereo = Vec::with_capacity(num_samples * 2);
        for i in 0..num_samples {
            let t = i as f32 / sample_rate;
            let left_sample = (2.0 * std::f32::consts::PI * left_freq * t).sin() * 0.8;
            let right_sample = (2.0 * std::f32::consts::PI * right_freq * t).sin() * 0.8;
            stereo.push(left_sample);
            stereo.push(right_sample);
        }

        let buffer = AudioBuffer {
            samples: stereo,
            channels: 2,
            sample_rate: 48000,
        };

        let result = analyzer.analyze(&buffer);

        // Left channel should detect ~200 Hz
        assert!(result.left_frequency.is_some());
        let detected_left = result.left_frequency.unwrap();
        assert!(
            (detected_left - 200.0).abs() < 50.0,
            "Expected ~200 Hz on left, got {} Hz",
            detected_left
        );

        // Right channel should detect ~800 Hz
        assert!(result.right_frequency.is_some());
        let detected_right = result.right_frequency.unwrap();
        assert!(
            (detected_right - 800.0).abs() < 50.0,
            "Expected ~800 Hz on right, got {} Hz",
            detected_right
        );

        // The frequencies should be different (this is the key test!)
        assert!(
            (detected_left - detected_right).abs() > 100.0,
            "Left ({} Hz) and right ({} Hz) should be significantly different",
            detected_left,
            detected_right
        );
    }

    #[test]
    fn test_calculate_rms() {
        // Silence
        assert_eq!(calculate_rms(&[]), 0.0);
        assert_eq!(calculate_rms(&[0.0, 0.0, 0.0]), 0.0);

        // DC offset (not typical audio but good test)
        let dc: Vec<f32> = vec![0.5; 100];
        let rms = calculate_rms(&dc);
        assert!(rms > 0.0 && rms < 1.0);
    }

    #[test]
    fn test_deinterleave_mono() {
        let analyzer = AudioAnalyzer::new(AnalysisMode::Amplitude, 48000);
        let mono_buffer = AudioBuffer {
            samples: vec![0.1, 0.2, 0.3],
            channels: 1,
            sample_rate: 48000,
        };
        let (left, right) = analyzer.deinterleave_stereo(&mono_buffer);
        assert_eq!(left, right);
        assert_eq!(left, mono_buffer.samples);
    }
}
