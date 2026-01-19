//! Application state and event handling for the TUI

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::audio::{AnalysisResult, FrequencyBands, MappingCurve as MapperMappingCurve, SPECTRUM_BARS};
use crate::ble::connection::ConnectionState;
use crate::ble::scanner::{CoyoteDevice, DeviceVersion};
use crate::config::Config;

/// Which panel is currently focused
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Panel {
    #[default]
    Devices,
    FrequencyBand,
    Parameters,
    Visualization,
}

impl Panel {
    pub fn next(self) -> Self {
        match self {
            Panel::Devices => Panel::FrequencyBand,
            Panel::FrequencyBand => Panel::Parameters,
            Panel::Parameters => Panel::Visualization,
            Panel::Visualization => Panel::Devices,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Panel::Devices => Panel::Visualization,
            Panel::FrequencyBand => Panel::Devices,
            Panel::Parameters => Panel::FrequencyBand,
            Panel::Visualization => Panel::Parameters,
        }
    }
}

/// Current input mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Editing,
}

/// Modal state for overlays
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalState {
    #[default]
    None,
    Help,
}

/// Mapping curve for audio to intensity conversion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MappingCurve {
    #[default]
    Linear,
    Exponential,
    Logarithmic,
    SCurve,
}

impl MappingCurve {
    pub fn next(self) -> Self {
        match self {
            MappingCurve::Linear => MappingCurve::Exponential,
            MappingCurve::Exponential => MappingCurve::Logarithmic,
            MappingCurve::Logarithmic => MappingCurve::SCurve,
            MappingCurve::SCurve => MappingCurve::Linear,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            MappingCurve::Linear => MappingCurve::SCurve,
            MappingCurve::Exponential => MappingCurve::Linear,
            MappingCurve::Logarithmic => MappingCurve::Exponential,
            MappingCurve::SCurve => MappingCurve::Logarithmic,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MappingCurve::Linear => "Linear",
            MappingCurve::Exponential => "Exponential",
            MappingCurve::Logarithmic => "Logarithmic",
            MappingCurve::SCurve => "S-Curve",
        }
    }

    /// Convert from the mapper's MappingCurve type
    pub fn from_mapper_curve(curve: MapperMappingCurve) -> Self {
        match curve {
            MapperMappingCurve::Linear => MappingCurve::Linear,
            MapperMappingCurve::Exponential => MappingCurve::Exponential,
            MapperMappingCurve::Logarithmic => MappingCurve::Logarithmic,
            MapperMappingCurve::SCurve => MappingCurve::SCurve,
        }
    }
}

/// Which parameter is selected in the Parameters panel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParameterSelection {
    #[default]
    MaxIntensityA,
    MaxIntensityB,
    Sensitivity,
    MappingCurve,
}

impl ParameterSelection {
    pub fn next(self) -> Self {
        match self {
            ParameterSelection::MaxIntensityA => ParameterSelection::MaxIntensityB,
            ParameterSelection::MaxIntensityB => ParameterSelection::Sensitivity,
            ParameterSelection::Sensitivity => ParameterSelection::MappingCurve,
            ParameterSelection::MappingCurve => ParameterSelection::MaxIntensityA,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ParameterSelection::MaxIntensityA => ParameterSelection::MappingCurve,
            ParameterSelection::MaxIntensityB => ParameterSelection::MaxIntensityA,
            ParameterSelection::Sensitivity => ParameterSelection::MaxIntensityB,
            ParameterSelection::MappingCurve => ParameterSelection::Sensitivity,
        }
    }
}

/// Which parameter is selected in the FrequencyBand panel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrequencyBandSelection {
    #[default]
    MinHz,
    MaxHz,
}

impl FrequencyBandSelection {
    pub fn next(self) -> Self {
        match self {
            FrequencyBandSelection::MinHz => FrequencyBandSelection::MaxHz,
            FrequencyBandSelection::MaxHz => FrequencyBandSelection::MinHz,
        }
    }

    pub fn prev(self) -> Self {
        self.next() // Only 2 options, prev == next
    }
}

/// Events that can be triggered by user input or system events
#[derive(Debug, Clone)]
pub enum AppEvent {
    Quit,
    ScanDevices,
    Connect(usize),
    Disconnect,
    SetMaxIntensityA(u16),
    SetMaxIntensityB(u16),
    SetSensitivity(f32),
    SetMappingCurve(MappingCurve),
    SetFreqBandMin(f32),
    SetFreqBandMax(f32),
    SaveConfig,
    EmergencyStop,
    TogglePause,
    RefreshDisplay,
}

/// Real-time output values for display
#[derive(Debug, Clone, Default)]
pub struct OutputValues {
    pub intensity_a: u16,
    pub intensity_b: u16,
    pub coyote_frequency_a: u16,
    pub coyote_frequency_b: u16,
    pub pulse_width: u8,
    pub detected_frequency_left: Option<f32>,
    pub detected_frequency_right: Option<f32>,
}

/// Application state
pub struct App {
    pub config: Config,
    pub active_panel: Panel,
    pub input_mode: InputMode,
    pub modal_state: ModalState,
    pub help_scroll_offset: u16,

    // Device panel state
    pub devices: Vec<CoyoteDevice>,
    pub selected_device: usize,
    pub connection_state: ConnectionState,
    pub connected_device_version: Option<DeviceVersion>,
    pub battery_level: Option<u8>,
    pub is_scanning: bool,

    // Frequency band panel state
    pub freq_band_selection: FrequencyBandSelection,

    // Parameters panel state
    pub parameter_selection: ParameterSelection,
    pub mapping_curve: MappingCurve,

    // Visualization state
    pub audio_levels: (f32, f32), // L/R amplitude 0.0-1.0
    pub frequency_bands: FrequencyBands, // Combined L/R frequency band energy
    pub spectrum_left: [f32; SPECTRUM_BARS], // Left channel spectrum
    pub spectrum_right: [f32; SPECTRUM_BARS], // Right channel spectrum
    pub output_values: OutputValues,
    pub beat_detected: bool,
    pub beat_flash_until: Option<Instant>,

    // Status
    pub status_message: Option<String>,
    pub error_message: Option<String>,

    // Control
    pub should_quit: bool,
    pub is_paused: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let mapping_curve = MappingCurve::from_mapper_curve(config.mapping_curve);
        Self {
            config,
            active_panel: Panel::default(),
            input_mode: InputMode::default(),
            modal_state: ModalState::default(),
            help_scroll_offset: 0,

            devices: Vec::new(),
            selected_device: 0,
            connection_state: ConnectionState::Disconnected,
            connected_device_version: None,
            battery_level: None,
            is_scanning: false,

            freq_band_selection: FrequencyBandSelection::default(),

            parameter_selection: ParameterSelection::default(),
            mapping_curve,

            audio_levels: (0.0, 0.0),
            frequency_bands: FrequencyBands::default(),
            spectrum_left: [0.0; SPECTRUM_BARS],
            spectrum_right: [0.0; SPECTRUM_BARS],
            output_values: OutputValues::default(),
            beat_detected: false,
            beat_flash_until: None,

            status_message: Some("Press 's' to scan for devices".to_string()),
            error_message: None,

            should_quit: false,
            is_paused: false,
        }
    }

    /// Poll for keyboard events with a timeout
    pub fn poll_event(&self, timeout: Duration) -> Option<KeyEvent> {
        if event::poll(timeout).ok()? {
            if let Event::Key(key) = event::read().ok()? {
                // Only handle key press events, not release or repeat
                // This is essential on Linux where crossterm fires multiple event kinds
                if key.kind == KeyEventKind::Press {
                    return Some(key);
                }
            }
        }
        None
    }

    /// Handle a key event and return any resulting app events
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<AppEvent> {
        let mut events = Vec::new();

        // Handle modal input first (takes priority)
        if self.modal_state != ModalState::None {
            self.handle_modal_key(key);
            return events;
        }

        // Global keys
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                events.push(AppEvent::SaveConfig);
                events.push(AppEvent::Quit);
                self.should_quit = true;
                return events;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                events.push(AppEvent::Quit);
                self.should_quit = true;
                return events;
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                events.push(AppEvent::RefreshDisplay);
                return events;
            }
            KeyCode::Esc => {
                // Emergency stop - set max intensity to 0
                events.push(AppEvent::EmergencyStop);
                self.status_message = Some("EMERGENCY STOP".to_string());
                return events;
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                // Toggle pause
                events.push(AppEvent::TogglePause);
                self.is_paused = !self.is_paused;
                if self.is_paused {
                    self.status_message = Some("Output paused".to_string());
                } else {
                    self.status_message = Some("Output resumed".to_string());
                }
                return events;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Toggle spectrum analyzer
                self.config.show_spectrum_analyzer = !self.config.show_spectrum_analyzer;
                if self.config.show_spectrum_analyzer {
                    self.status_message = Some("Spectrum analyzer enabled".to_string());
                } else {
                    self.status_message = Some("Spectrum analyzer disabled".to_string());
                }
                events.push(AppEvent::SaveConfig);
                return events;
            }
            KeyCode::Char('?') => {
                self.modal_state = ModalState::Help;
                self.help_scroll_offset = 0;
                return events;
            }
            KeyCode::Tab => {
                self.active_panel = self.active_panel.next();
                return events;
            }
            KeyCode::BackTab => {
                self.active_panel = self.active_panel.prev();
                return events;
            }
            _ => {}
        }

        // Panel-specific keys
        match self.active_panel {
            Panel::Devices => self.handle_devices_key(key, &mut events),
            Panel::FrequencyBand => self.handle_freq_band_key(key, &mut events),
            Panel::Parameters => self.handle_parameters_key(key, &mut events),
            Panel::Visualization => {} // Read-only panel
        }

        events
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        match self.modal_state {
            ModalState::Help => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                        self.modal_state = ModalState::None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.help_scroll_offset = self.help_scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.help_scroll_offset = self.help_scroll_offset.saturating_add(1);
                    }
                    KeyCode::PageUp => {
                        self.help_scroll_offset = self.help_scroll_offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        self.help_scroll_offset = self.help_scroll_offset.saturating_add(10);
                    }
                    KeyCode::Home => {
                        self.help_scroll_offset = 0;
                    }
                    _ => {}
                }
            }
            ModalState::None => {}
        }
    }

    fn handle_devices_key(&mut self, key: KeyEvent, events: &mut Vec<AppEvent>) {
        match key.code {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if !self.is_scanning {
                    events.push(AppEvent::ScanDevices);
                    self.is_scanning = true;
                    self.status_message = Some("Scanning for devices...".to_string());
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_device > 0 {
                    self.selected_device -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.devices.is_empty() && self.selected_device < self.devices.len() - 1 {
                    self.selected_device += 1;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if self.connection_state == ConnectionState::Connected {
                    events.push(AppEvent::Disconnect);
                    self.status_message = Some("Disconnecting...".to_string());
                } else if !self.devices.is_empty() {
                    events.push(AppEvent::Connect(self.selected_device));
                    self.status_message = Some("Connecting...".to_string());
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if self.connection_state == ConnectionState::Connected {
                    events.push(AppEvent::Disconnect);
                    self.status_message = Some("Disconnecting...".to_string());
                }
            }
            _ => {}
        }
    }

    fn handle_freq_band_key(&mut self, key: KeyEvent, events: &mut Vec<AppEvent>) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.freq_band_selection = self.freq_band_selection.prev();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.freq_band_selection = self.freq_band_selection.next();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.adjust_freq_band(-1, events);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.adjust_freq_band(1, events);
            }
            KeyCode::Char('[') => {
                self.adjust_freq_band(-10, events);
            }
            KeyCode::Char(']') => {
                self.adjust_freq_band(10, events);
            }
            _ => {}
        }
    }

    fn adjust_freq_band(&mut self, delta: i32, events: &mut Vec<AppEvent>) {
        let step = 10.0; // 10 Hz per step
        match self.freq_band_selection {
            FrequencyBandSelection::MinHz => {
                let current = self.config.freq_band_min;
                let new_val = current + delta as f32 * step;
                self.config.set_freq_band_min(new_val);
                events.push(AppEvent::SetFreqBandMin(self.config.freq_band_min));
            }
            FrequencyBandSelection::MaxHz => {
                let current = self.config.freq_band_max;
                let new_val = current + delta as f32 * step;
                self.config.set_freq_band_max(new_val);
                events.push(AppEvent::SetFreqBandMax(self.config.freq_band_max));
            }
        }
    }

    fn handle_parameters_key(&mut self, key: KeyEvent, events: &mut Vec<AppEvent>) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.parameter_selection = self.parameter_selection.prev();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.parameter_selection = self.parameter_selection.next();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.adjust_parameter(-1, events);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.adjust_parameter(1, events);
            }
            KeyCode::Char('[') => {
                self.adjust_parameter(-10, events);
            }
            KeyCode::Char(']') => {
                self.adjust_parameter(10, events);
            }
            _ => {}
        }
    }

    fn adjust_parameter(&mut self, delta: i32, events: &mut Vec<AppEvent>) {
        match self.parameter_selection {
            ParameterSelection::MaxIntensityA => {
                let current = self.config.max_intensity_a as i32;
                let new_val = (current + delta * 20).clamp(0, 2047) as u16;
                self.config.set_max_intensity_a(new_val);
                events.push(AppEvent::SetMaxIntensityA(new_val));
            }
            ParameterSelection::MaxIntensityB => {
                let current = self.config.max_intensity_b as i32;
                let new_val = (current + delta * 20).clamp(0, 2047) as u16;
                self.config.set_max_intensity_b(new_val);
                events.push(AppEvent::SetMaxIntensityB(new_val));
            }
            ParameterSelection::Sensitivity => {
                let current = self.config.sensitivity;
                let new_val = (current + delta as f32 * 0.05).clamp(0.0, 1.0);
                self.config.set_sensitivity(new_val);
                events.push(AppEvent::SetSensitivity(new_val));
            }
            ParameterSelection::MappingCurve => {
                if delta > 0 {
                    self.mapping_curve = self.mapping_curve.next();
                } else {
                    self.mapping_curve = self.mapping_curve.prev();
                }
                events.push(AppEvent::SetMappingCurve(self.mapping_curve));
            }
        }
    }

    /// Update audio visualization from analysis result
    pub fn update_audio(&mut self, result: &AnalysisResult) {
        self.audio_levels = (result.left.amplitude, result.right.amplitude);
        self.beat_detected = result.beat_detected;
        self.spectrum_left = result.spectrum_left;
        self.spectrum_right = result.spectrum_right;

        // Average the frequency bands from left and right channels
        self.frequency_bands = FrequencyBands {
            bass: (result.left.frequency_bands.bass + result.right.frequency_bands.bass) / 2.0,
            mid: (result.left.frequency_bands.mid + result.right.frequency_bands.mid) / 2.0,
            treble: (result.left.frequency_bands.treble + result.right.frequency_bands.treble) / 2.0,
        };

        if result.beat_detected {
            self.beat_flash_until = Some(Instant::now() + Duration::from_millis(100));
        }
    }

    /// Check if beat indicator should still be flashing
    pub fn is_beat_flashing(&self) -> bool {
        self.beat_flash_until
            .map(|until| Instant::now() < until)
            .unwrap_or(false)
    }

    /// Update output values for visualization
    pub fn update_output(&mut self, values: OutputValues) {
        self.output_values = values;
    }

    /// Update device list after scanning
    pub fn update_devices(&mut self, devices: Vec<CoyoteDevice>) {
        self.devices = devices;
        self.is_scanning = false;
        if self.devices.is_empty() {
            self.status_message = Some("No devices found. Press 's' to scan again.".to_string());
        } else {
            self.status_message = Some(format!("Found {} device(s)", self.devices.len()));
        }
        self.selected_device = 0;
    }

    /// Update connection state
    pub fn update_connection_state(&mut self, state: ConnectionState) {
        self.connection_state = state;
        match state {
            ConnectionState::Disconnected => {
                self.status_message = Some("Disconnected".to_string());
                self.battery_level = None;
                self.connected_device_version = None;
            }
            ConnectionState::Connecting => {
                self.status_message = Some("Connecting...".to_string());
            }
            ConnectionState::Connected => {
                self.status_message = Some("Connected".to_string());
            }
            ConnectionState::Reconnecting => {
                self.status_message = Some("Reconnecting...".to_string());
            }
        }
    }

    /// Set the connected device version
    pub fn set_connected_device_version(&mut self, version: DeviceVersion) {
        self.connected_device_version = Some(version);
    }

    /// Update battery level
    pub fn update_battery(&mut self, level: u8) {
        self.battery_level = Some(level);
    }

    /// Set an error message
    pub fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
    }

    /// Clear error message
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Get max intensity as percentage (0-100) for display
    pub fn max_intensity_a_percent(&self) -> u8 {
        ((self.config.max_intensity_a as f32 / 2047.0) * 100.0) as u8
    }

    /// Get max intensity as percentage (0-100) for display
    pub fn max_intensity_b_percent(&self) -> u8 {
        ((self.config.max_intensity_b as f32 / 2047.0) * 100.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_cycle() {
        let panel = Panel::Devices;
        assert_eq!(panel.next(), Panel::FrequencyBand);
        assert_eq!(panel.next().next(), Panel::Parameters);
        assert_eq!(panel.next().next().next(), Panel::Visualization);
        assert_eq!(panel.next().next().next().next(), Panel::Devices);
    }

    #[test]
    fn test_panel_cycle_backwards() {
        let panel = Panel::Devices;
        assert_eq!(panel.prev(), Panel::Visualization);
        assert_eq!(panel.prev().prev(), Panel::Parameters);
    }

    #[test]
    fn test_mapping_curve_cycle() {
        let curve = MappingCurve::Linear;
        assert_eq!(curve.next(), MappingCurve::Exponential);
        assert_eq!(curve.next().next().next().next(), MappingCurve::Linear);
    }

    #[test]
    fn test_parameter_selection_cycle() {
        let sel = ParameterSelection::MaxIntensityA;
        assert_eq!(sel.next(), ParameterSelection::MaxIntensityB);
        assert_eq!(sel.next().next(), ParameterSelection::Sensitivity);
        assert_eq!(sel.next().next().next(), ParameterSelection::MappingCurve);
        assert_eq!(sel.next().next().next().next(), ParameterSelection::MaxIntensityA);
    }

    #[test]
    fn test_app_intensity_percent() {
        let mut config = Config::default();
        config.max_intensity_a = 2047;
        config.max_intensity_b = 1024;
        let app = App::new(config);

        assert_eq!(app.max_intensity_a_percent(), 100);
        assert_eq!(app.max_intensity_b_percent(), 50);
    }

    #[test]
    fn test_beat_flash() {
        let config = Config::default();
        let mut app = App::new(config);

        assert!(!app.is_beat_flashing());

        app.beat_flash_until = Some(Instant::now() + Duration::from_secs(1));
        assert!(app.is_beat_flashing());

        app.beat_flash_until = Some(Instant::now() - Duration::from_secs(1));
        assert!(!app.is_beat_flashing());
    }
}
