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
    // Initialize logging to file to avoid interfering with TUI
    let log_file = std::fs::File::create("/tmp/coyote-audio.log")
        .expect("Failed to create log file");
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(Box::new(log_file)))
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

    // Main loop timing - use Delay behavior to skip missed ticks instead of bursting
    let frame_duration = Duration::from_millis(33); // ~30 fps
    let mut frame_interval = interval(frame_duration);
    frame_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ble_interval = interval(Duration::from_millis(COMMAND_INTERVAL_MS));
    ble_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Run the main loop
    loop {
        tokio::select! {
            // TUI frame tick
            _ = frame_interval.tick() => {
                // Process pending audio buffers, keeping only the most recent
                // to avoid latency from buffered audio
                let mut got_new_audio = false;
                let mut new_analysis: Option<AnalysisResult> = None;
                if let Some(ref rx) = audio_rx {
                    // Drain all pending buffers, only analyze the last one
                    let mut latest_buffer = None;
                    while let Ok(buffer) = rx.try_recv() {
                        latest_buffer = Some(buffer);
                        got_new_audio = true;
                    }
                    if let Some(buffer) = latest_buffer {
                        new_analysis = Some(analyzer.analyze(&buffer));
                    }
                }

                // Get or reuse the last analysis result
                let analysis_result = {
                    let state = shared_state.lock().await;
                    new_analysis.or_else(|| state.analysis.clone())
                };

                // Recalculate command with current config (handles setting changes immediately)
                if let Some(ref analysis) = analysis_result {
                    let command = mapper.map(analysis, &app.config);

                    // Update app visualization
                    app.update_audio(analysis);
                    app.update_output(OutputValues {
                        intensity_a: command.intensity.channel_a,
                        intensity_b: command.intensity.channel_b,
                        coyote_frequency_a: mapper.last_coyote_freq_a(),
                        coyote_frequency_b: mapper.last_coyote_freq_b(),
                        pulse_width: command.waveform_a.params.z,
                        detected_frequency_left: analysis.left_frequency,
                        detected_frequency_right: analysis.right_frequency,
                    });

                    // Store for BLE sending
                    let mut state = shared_state.lock().await;
                    if got_new_audio {
                        state.analysis = analysis_result;
                    }
                    state.command = Some(command);
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
                            AppEvent::RefreshDisplay => {
                                // Clear and redraw the entire terminal
                                let _ = terminal.clear();
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

                // Get the command to send before borrowing connection mutably
                let cmd_to_send = if app.is_paused {
                    Some(CoyoteCommand::default())
                } else {
                    state.command.clone()
                };
                let has_connection = state.connection.is_some();

                if let Some(ref cmd) = cmd_to_send {
                    log::debug!(
                        "BLE tick: paused={} connected={} intensity=({},{}) sending...",
                        app.is_paused, has_connection,
                        cmd.intensity.channel_a, cmd.intensity.channel_b
                    );
                }

                // Send command if connected
                if has_connection {
                    if let Some(ref cmd) = cmd_to_send {
                        if let Some(ref mut connection) = state.connection {
                            let send_start = std::time::Instant::now();
                            match send_ble_command(connection, cmd, &app.config).await {
                                Ok(()) => {
                                    let elapsed = send_start.elapsed();
                                    if elapsed.as_millis() > 50 {
                                        log::warn!("BLE send took {}ms", elapsed.as_millis());
                                    }
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
                            let device_version = app.devices[idx].version;

                            // Get adapter from scanner to pass to connection
                            if let Some(ref scanner) = scanner {
                                let adapter = scanner.adapter().clone();

                                log::info!("Connecting to {} device: {}", device_version, device_address);
                                app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Connecting);

                                let mut conn = CoyoteConnection::new(adapter, device_address.clone(), device_version);
                                match conn.connect_with_retry().await {
                                    Ok(()) => {
                                        log::info!("Connected to {} ({})", device_address, device_version);
                                        app.update_connection_state(coyote_audio::ble::connection::ConnectionState::Connected);

                                        // Store device version for display
                                        app.set_connected_device_version(device_version);

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
                            } else {
                                app.set_error("BLE not available".to_string());
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
    connection: &mut CoyoteConnection,
    command: &CoyoteCommand,
    _config: &Config,
) -> Result<(), String> {
    // Check if still connected
    match connection.is_connected().await {
        Ok(true) => {}
        Ok(false) => return Err("Disconnected".to_string()),
        Err(e) => return Err(format!("Connection check failed: {}", e)),
    }

    // Fixed waveform params: X=1 (single pulse), Z=20 (100us pulse width)
    connection.set_waveform_params(1, 20);

    // Use per-channel Y from the mapper (audio frequency controls output frequency)
    // freq = X + Y, where Y is calculated from audio frequency
    // Left audio freq -> Channel A, Right audio freq -> Channel B
    let freq_a = 1 + command.waveform_a.params.y;  // X=1
    let freq_b = 1 + command.waveform_b.params.y;  // X=1

    // Send command with per-channel frequencies
    connection
        .send_command(
            command.intensity.channel_a,
            command.intensity.channel_b,
            freq_a,
            freq_b,
        )
        .await
        .map_err(|e| e.to_string())?;

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
