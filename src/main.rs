//! Coyote Audio - Audio-reactive control for the Coyote V2 device
//!
//! This application captures audio from PipeWire, analyzes it, and maps the
//! audio characteristics to stimulation parameters sent to a Coyote V2 device
//! over Bluetooth Low Energy.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::Mutex;
use tokio::time::interval;

use coyote_audio::audio::{
    AnalysisMode, AnalysisResult, AudioAnalyzer, AudioCapture, AudioCaptureConfig, AudioMapper,
    CoyoteCommand, MappingCurve as MapperMappingCurve,
};
use coyote_audio::ble::{CoyoteConnection, CoyoteScanner, COMMAND_INTERVAL_MS};
use coyote_audio::config::Config;
use coyote_audio::tui::{draw, App, AppEvent, MappingCurve, OutputValues};

/// Application error type
#[derive(Debug)]
enum AppError {
    Io(std::io::Error),
    Config(coyote_audio::config::ConfigError),
    Audio(coyote_audio::audio::AudioCaptureError),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Io(e) => write!(f, "IO error: {}", e),
            AppError::Config(e) => write!(f, "Config error: {}", e),
            AppError::Audio(e) => write!(f, "Audio error: {}", e),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e)
    }
}

impl From<coyote_audio::config::ConfigError> for AppError {
    fn from(e: coyote_audio::config::ConfigError) -> Self {
        AppError::Config(e)
    }
}

impl From<coyote_audio::audio::AudioCaptureError> for AppError {
    fn from(e: coyote_audio::audio::AudioCaptureError) -> Self {
        AppError::Audio(e)
    }
}

/// Shared application state that can be accessed from multiple tasks
struct SharedState {
    /// Latest analysis result from audio processing
    analysis: Option<AnalysisResult>,
    /// Latest command to send to the device
    command: Option<CoyoteCommand>,
    /// BLE connection (if connected)
    connection: Option<CoyoteConnection>,
    /// Whether we should attempt reconnection
    reconnect_requested: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            analysis: None,
            command: None,
            connection: None,
            reconnect_requested: false,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .init();

    log::info!("Starting Coyote Audio");

    // Load configuration
    let config = Config::load().unwrap_or_else(|e| {
        log::warn!("Failed to load config: {}, using defaults", e);
        Config::default()
    });

    // Run the application
    let result = run_app(config).await;

    // Report any errors
    if let Err(ref e) = result {
        log::error!("Application error: {}", e);
    }

    result
}

async fn run_app(config: Config) -> Result<(), AppError> {
    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    execute!(stdout, Clear(ClearType::All))?;
    execute!(stdout, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create application state
    let mut app = App::new(config);
    let shared_state = Arc::new(Mutex::new(SharedState::default()));

    // Initialize BLE scanner
    let scanner = match CoyoteScanner::new().await {
        Ok(s) => Some(s),
        Err(e) => {
            log::error!("Failed to initialize BLE: {}", e);
            app.set_error(format!("BLE init failed: {}", e));
            None
        }
    };

    // Initialize audio capture
    let audio_config = AudioCaptureConfig::default();
    let mut audio_capture = AudioCapture::new(audio_config.clone());
    let audio_rx = match audio_capture.start() {
        Ok(rx) => Some(rx),
        Err(e) => {
            log::error!("Failed to start audio capture: {}", e);
            app.set_error(format!("Audio init failed: {}", e));
            None
        }
    };

    // Initialize audio analyzer and mapper
    // Analyzer always computes both amplitude and dominant frequency regardless of mode
    let mut analyzer = AudioAnalyzer::new(AnalysisMode::Amplitude, audio_config.sample_rate);
    let mut mapper = AudioMapper::new();
    mapper.set_intensity_curve(app.config.mapping_curve);

    // Event channel for app events that need async processing
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<AppEvent>(32);

    // Main loop timing
    let frame_duration = Duration::from_millis(33); // ~30 fps
    let mut frame_interval = interval(frame_duration);
    let mut ble_interval = interval(Duration::from_millis(COMMAND_INTERVAL_MS));

    // Run the main loop
    loop {
        tokio::select! {
            // TUI frame tick
            _ = frame_interval.tick() => {
                // Process any pending audio buffers
                if let Some(ref rx) = audio_rx {
                    while let Ok(buffer) = rx.try_recv() {
                        let analysis_result = analyzer.analyze(&buffer);
                        let command = mapper.map(&analysis_result, &app.config);

                        // Update app visualization
                        app.update_audio(&analysis_result);
                        app.update_output(OutputValues {
                            intensity_a: command.intensity.channel_a,
                            intensity_b: command.intensity.channel_b,
                            coyote_frequency_a: mapper.last_coyote_freq_a(),
                            coyote_frequency_b: mapper.last_coyote_freq_b(),
                            pulse_width: command.waveform_a.params.z,
                            detected_frequency_left: analysis_result.left_frequency,
                            detected_frequency_right: analysis_result.right_frequency,
                        });

                        // Store for BLE sending
                        let mut state = shared_state.lock().await;
                        state.analysis = Some(analysis_result);
                        state.command = Some(command);
                    }
                }

                // Poll for keyboard input
                if let Some(key) = app.poll_event(Duration::from_millis(0)) {
                    let events = app.handle_key(key);

                    // Check if we should quit (handle_key sets this flag)
                    if app.should_quit {
                        // Process SaveConfig if it was queued before Quit
                        for event in &events {
                            if let AppEvent::SaveConfig = event {
                                if let Err(e) = app.config.save() {
                                    log::error!("Failed to save config: {}", e);
                                }
                            }
                        }
                        break;
                    }

                    // Process events
                    for event in events {
                        match event {
                            AppEvent::SaveConfig => {
                                if let Err(e) = app.config.save() {
                                    log::error!("Failed to save config: {}", e);
                                }
                            }
                            AppEvent::SetMappingCurve(curve) => {
                                let mapper_curve = tui_curve_to_mapper_curve(curve);
                                mapper.set_intensity_curve(mapper_curve);
                                app.config.set_mapping_curve(mapper_curve);
                            }
                            AppEvent::EmergencyStop => {
                                // Set max intensity to 0 for both channels
                                app.config.set_max_intensity_a(0);
                                app.config.set_max_intensity_b(0);
                                // Clear the command to send zero immediately
                                let mut state = shared_state.lock().await;
                                state.command = None;
                                log::warn!("EMERGENCY STOP activated");
                            }
                            AppEvent::TogglePause => {
                                // is_paused is already toggled in app.handle_key()
                                // Log the state change
                                if app.is_paused {
                                    log::info!("Output paused");
                                } else {
                                    log::info!("Output resumed");
                                }
                            }
                            _ => {
                                // Send other events for async processing
                                let _ = event_tx.send(event).await;
                            }
                        }
                    }
                }

                // Render the UI
                terminal.draw(|f| draw(f, &app))?;
            }

            // BLE command tick
            _ = ble_interval.tick() => {
                let mut state = shared_state.lock().await;

                // Send command if connected
                if let Some(ref connection) = state.connection {
                    // If paused, send zero output; otherwise send the computed command
                    let cmd_to_send = if app.is_paused {
                        // Create a zero-output command when paused
                        Some(CoyoteCommand::default())
                    } else {
                        state.command.clone()
                    };

                    if let Some(ref cmd) = cmd_to_send {
                        // Try to send the command
                        match send_ble_command(connection, cmd).await {
                            Ok(()) => {
                                app.clear_error();
                            }
                            Err(e) => {
                                log::warn!("BLE send error: {}", e);
                                app.set_error(format!("BLE: {}", e));
                                // Mark for reconnection
                                state.reconnect_requested = true;
                            }
                        }
                    }
                }

                // Handle reconnection
                if state.reconnect_requested {
                    if let Some(mut conn) = state.connection.take() {
                        log::info!("Attempting BLE reconnection");
                        app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Reconnecting);

                        match conn.reconnect().await {
                            Ok(()) => {
                                log::info!("Reconnected successfully");
                                app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Connected);
                                state.connection = Some(conn);
                                state.reconnect_requested = false;
                            }
                            Err(e) => {
                                log::error!("Reconnection failed: {}", e);
                                app.set_error(format!("Reconnect failed: {}", e));
                                app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Disconnected);
                                state.reconnect_requested = false;
                            }
                        }
                    }
                }
            }

            // Async event processing
            Some(event) = event_rx.recv() => {
                match event {
                    AppEvent::ScanDevices => {
                        if let Some(ref scanner) = scanner {
                            log::info!("Starting BLE scan");
                            match scanner.scan_for_devices(Duration::from_secs(10)).await {
                                Ok(devices) => {
                                    log::info!("Found {} devices", devices.len());
                                    app.update_devices(devices);
                                }
                                Err(e) => {
                                    log::error!("Scan failed: {}", e);
                                    app.set_error(format!("Scan failed: {}", e));
                                    app.is_scanning = false;
                                }
                            }
                        } else {
                            app.set_error("BLE not available".to_string());
                            app.is_scanning = false;
                        }
                    }

                    AppEvent::Connect(idx) => {
                        if idx < app.devices.len() {
                            // Clone what we need from the device to avoid borrow conflicts
                            let device_address = app.devices[idx].address.clone();
                            let device_peripheral = app.devices[idx].peripheral.clone();

                            log::info!("Connecting to device: {}", device_address);
                            app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Connecting);

                            let mut conn = CoyoteConnection::new(device_peripheral);
                            match conn.connect_with_retry().await {
                                Ok(()) => {
                                    log::info!("Connected to {}", device_address);
                                    app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Connected);

                                    // Store connection and device address
                                    let mut state = shared_state.lock().await;
                                    state.connection = Some(conn);

                                    // Save last device address
                                    app.config.last_device_address = Some(device_address);

                                    // Reset mapper ramp for new connection
                                    mapper.reset_ramp();
                                }
                                Err(e) => {
                                    log::error!("Connection failed: {}", e);
                                    app.set_error(format!("Connect failed: {}", e));
                                    app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Disconnected);
                                }
                            }
                        }
                    }

                    AppEvent::Disconnect => {
                        let mut state = shared_state.lock().await;
                        if let Some(mut conn) = state.connection.take() {
                            log::info!("Disconnecting");
                            if let Err(e) = conn.disconnect().await {
                                log::warn!("Disconnect error: {}", e);
                            }
                        }
                        app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Disconnected);
                    }

                    _ => {
                        // Other events handled synchronously above
                    }
                }
            }
        }
    }

    // Cleanup
    log::info!("Shutting down");

    // Save config on exit
    if let Err(e) = app.config.save() {
        log::warn!("Failed to save config on exit: {}", e);
    }

    // FIRST: Restore terminal so user can see their shell even if cleanup hangs
    log::info!("Restoring terminal");
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();

    // Disconnect BLE gracefully with timeout
    {
        let disconnect_future = async {
            let mut state = shared_state.lock().await;
            if let Some(mut conn) = state.connection.take() {
                log::info!("Disconnecting BLE on shutdown");
                let _ = conn.disconnect().await;
            }
        };
        // Give BLE disconnect 2 seconds max
        let _ = tokio::time::timeout(Duration::from_secs(2), disconnect_future).await;
    }

    // Stop audio capture (this will signal the PipeWire thread to exit)
    log::info!("Stopping audio capture");
    drop(audio_capture);

    log::info!("Shutdown complete");

    // Force exit to ensure we don't hang on any remaining async tasks
    std::process::exit(0);
}

/// Send BLE command to the connected device
async fn send_ble_command(
    connection: &CoyoteConnection,
    command: &CoyoteCommand,
) -> Result<(), String> {
    // Check if still connected
    match connection.is_connected().await {
        Ok(true) => {}
        Ok(false) => return Err("Disconnected".to_string()),
        Err(e) => return Err(format!("Connection check failed: {}", e)),
    }

    // Send intensity
    connection
        .set_intensity(command.intensity)
        .await
        .map_err(|e| format!("Set intensity: {}", e))?;

    // Send waveforms
    connection
        .set_waveform_a(command.waveform_a)
        .await
        .map_err(|e| format!("Set waveform A: {}", e))?;

    connection
        .set_waveform_b(command.waveform_b)
        .await
        .map_err(|e| format!("Set waveform B: {}", e))?;

    Ok(())
}

/// Convert TUI MappingCurve to mapper MappingCurve
fn tui_curve_to_mapper_curve(curve: MappingCurve) -> MapperMappingCurve {
    match curve {
        MappingCurve::Linear => MapperMappingCurve::Linear,
        MappingCurve::Exponential => MapperMappingCurve::Exponential,
        MappingCurve::Logarithmic => MapperMappingCurve::Logarithmic,
        MappingCurve::SCurve => MapperMappingCurve::SCurve,
    }
}
