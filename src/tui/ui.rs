//! Widget rendering for the TUI

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Tabs,
};
use ratatui::Frame;

use crate::audio::SPECTRUM_BARS;
use crate::ble::connection::ConnectionState;

use super::app::{App, FrequencyBandSelection, ModalState, Panel, ParameterSelection};

/// Main draw function - renders all UI components
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Tab bar
            Constraint::Min(10),    // Main content
            Constraint::Length(3),  // Status bar
        ])
        .split(frame.area());

    draw_tab_bar(frame, app, chunks[0]);
    draw_main_content(frame, app, chunks[1]);
    draw_status_bar(frame, app, chunks[2]);

    // Draw modal overlays on top
    match app.modal_state {
        ModalState::Help => draw_help_modal(frame, app),
        ModalState::None => {}
    }
}

fn draw_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let titles = vec!["Devices", "Freq Band", "Parameters", "Visualization"];
    let selected = match app.active_panel {
        Panel::Devices => 0,
        Panel::FrequencyBand => 1,
        Panel::Parameters => 2,
        Panel::Visualization => 3,
    };

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Coyote Audio"))
        .select(selected)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider(symbols::DOT);

    frame.render_widget(tabs, area);
}

fn draw_main_content(frame: &mut Frame, app: &App, area: Rect) {
    // Split into left and right columns
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left column: Devices and Frequency Band panels
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[0]);

    draw_devices_panel(frame, app, left_chunks[0]);
    draw_freq_band_panel(frame, app, left_chunks[1]);

    // Right column: Parameters and Visualization panels
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    draw_parameters_panel(frame, app, right_chunks[0]);
    draw_visualization_panel(frame, app, right_chunks[1]);
}

fn panel_style(app: &App, panel: Panel) -> Style {
    if app.active_panel == panel {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

fn draw_devices_panel(frame: &mut Frame, app: &App, area: Rect) {
    let is_active = app.active_panel == Panel::Devices;

    let mut title = String::from("Devices");
    if app.is_scanning {
        title.push_str(" [Scanning...]");
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(panel_style(app, Panel::Devices));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area for device list and connection info
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(4)])
        .split(inner_area);

    // Device list
    let items: Vec<ListItem> = if app.devices.is_empty() {
        vec![ListItem::new(Line::from(vec![
            Span::styled("No devices found", Style::default().fg(Color::DarkGray)),
        ]))]
    } else {
        app.devices
            .iter()
            .enumerate()
            .map(|(i, device)| {
                let is_selected = i == app.selected_device;
                let is_connected = app.connection_state == ConnectionState::Connected && is_selected;

                let mut spans = vec![];

                // Selection indicator
                if is_selected && is_active {
                    spans.push(Span::styled("> ", Style::default().fg(Color::Yellow)));
                } else {
                    spans.push(Span::raw("  "));
                }

                // Device name with version
                let name_style = if is_connected {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else if is_selected && is_active {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(format!("{} [{}]", &device.name, device.version), name_style));

                // Address
                spans.push(Span::styled(
                    format!(" ({})", &device.address),
                    Style::default().fg(Color::DarkGray),
                ));

                // Connection status with battery
                if is_connected {
                    spans.push(Span::styled(" [Connected]", Style::default().fg(Color::Green)));
                    if let Some(level) = app.battery_level {
                        let battery_color = if level > 50 {
                            Color::Green
                        } else if level > 20 {
                            Color::Yellow
                        } else {
                            Color::Red
                        };
                        spans.push(Span::styled(
                            format!("  Battery {}%", level),
                            Style::default().fg(battery_color),
                        ));
                    }
                }

                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let list = List::new(items);
    frame.render_widget(list, inner_chunks[0]);

    // Connection info
    let conn_info = build_connection_info(app);
    let info_paragraph = Paragraph::new(conn_info);
    frame.render_widget(info_paragraph, inner_chunks[1]);
}

fn build_connection_info(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![];

    // Connection state with version when connected
    let (state_text, state_color) = match app.connection_state {
        ConnectionState::Disconnected => ("Disconnected".to_string(), Color::Red),
        ConnectionState::Connecting => ("Connecting...".to_string(), Color::Yellow),
        ConnectionState::Connected => {
            if let Some(version) = app.connected_device_version {
                (format!("Connected ({})", version), Color::Green)
            } else {
                ("Connected".to_string(), Color::Green)
            }
        }
        ConnectionState::Reconnecting => ("Reconnecting...".to_string(), Color::Yellow),
    };
    lines.push(Line::from(vec![
        Span::raw("Status: "),
        Span::styled(state_text, Style::default().fg(state_color)),
    ]));

    // Battery level
    if let Some(level) = app.battery_level {
        let battery_color = if level > 50 {
            Color::Green
        } else if level > 20 {
            Color::Yellow
        } else {
            Color::Red
        };
        lines.push(Line::from(vec![
            Span::raw("Battery: "),
            Span::styled(format!("{}%", level), Style::default().fg(battery_color)),
        ]));
    }

    // Help text
    lines.push(Line::from(Span::styled(
        "[s]can [Enter]connect [d]isconnect",
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

fn draw_freq_band_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Frequency Band")
        .border_style(panel_style(app, Panel::FrequencyBand));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let is_active = app.active_panel == Panel::FrequencyBand;

    // Split for min/max sliders and info
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Min Hz
            Constraint::Length(2), // Max Hz
            Constraint::Length(2), // Detected freq display
            Constraint::Min(0),    // Info text
        ])
        .split(inner_area);

    // Min Hz slider
    draw_freq_slider(
        frame,
        chunks[0],
        "Min Hz",
        app.config.freq_band_min,
        20.0,
        2000.0,
        is_active && app.freq_band_selection == FrequencyBandSelection::MinHz,
        Color::Cyan,
    );

    // Max Hz slider
    draw_freq_slider(
        frame,
        chunks[1],
        "Max Hz",
        app.config.freq_band_max,
        20.0,
        2000.0,
        is_active && app.freq_band_selection == FrequencyBandSelection::MaxHz,
        Color::Magenta,
    );

    // Detected frequency display (L/R)
    let left_text = match app.output_values.detected_frequency_left {
        Some(freq) => format!("{:.0}", freq),
        None => "--".to_string(),
    };
    let right_text = match app.output_values.detected_frequency_right {
        Some(freq) => format!("{:.0}", freq),
        None => "--".to_string(),
    };
    let detected_line = Line::from(vec![
        Span::raw("  Detected: L "),
        Span::styled(left_text, Style::default().fg(Color::Cyan)),
        Span::raw(" Hz  R "),
        Span::styled(right_text, Style::default().fg(Color::Magenta)),
        Span::raw(" Hz"),
    ]);
    frame.render_widget(Paragraph::new(detected_line), chunks[2]);

    // Help text
    let help_text = Line::from(Span::styled(
        "  Audio freq in this range -> Coyote freq (10-100)",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(help_text), chunks[3]);
}

fn draw_freq_slider(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: f32,
    min: f32,
    max: f32,
    is_selected: bool,
    color: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(10),
            Constraint::Length(8),
        ])
        .split(area);

    // Label
    let label_style = if is_selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let prefix = if is_selected { "> " } else { "  " };
    let label_text = format!("{}{}", prefix, label);
    frame.render_widget(Paragraph::new(label_text).style(label_style), chunks[0]);

    // Gauge
    let gauge_color = if is_selected { Color::Yellow } else { color };
    let ratio = ((value - min) / (max - min)) as f64;
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color))
        .ratio(ratio.clamp(0.0, 1.0));
    frame.render_widget(gauge, chunks[1]);

    // Value
    let value_text = format!("{:>5.0} Hz", value);
    frame.render_widget(Paragraph::new(value_text).style(label_style), chunks[2]);
}

fn draw_parameters_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Parameters")
        .border_style(panel_style(app, Panel::Parameters));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let is_active = app.active_panel == Panel::Parameters;

    // Split for each parameter
    let param_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Max Intensity A
            Constraint::Length(2),  // Max Intensity B
            Constraint::Min(0),     // Remaining space
        ])
        .split(inner_area);

    // Max Intensity A
    draw_slider(
        frame,
        param_chunks[0],
        "Max Intensity A",
        app.max_intensity_a_percent() as u16,
        100,
        is_active && app.parameter_selection == ParameterSelection::MaxIntensityA,
        Color::Cyan,
    );

    // Max Intensity B
    draw_slider(
        frame,
        param_chunks[1],
        "Max Intensity B",
        app.max_intensity_b_percent() as u16,
        100,
        is_active && app.parameter_selection == ParameterSelection::MaxIntensityB,
        Color::Magenta,
    );
}

fn draw_slider(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: u16,
    max: u16,
    is_selected: bool,
    color: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Min(10), Constraint::Length(6)])
        .split(area);

    // Label
    let label_style = if is_selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let prefix = if is_selected { "> " } else { "  " };
    let label_text = format!("{}{}", prefix, label);
    frame.render_widget(
        Paragraph::new(label_text).style(label_style),
        chunks[0],
    );

    // Gauge
    let gauge_color = if is_selected { Color::Yellow } else { color };
    let ratio = (value as f64) / (max as f64);
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color))
        .ratio(ratio.min(1.0));
    frame.render_widget(gauge, chunks[1]);

    // Value
    let value_text = format!("{:>3}%", value.min(100));
    frame.render_widget(
        Paragraph::new(value_text).style(label_style),
        chunks[2],
    );
}

fn draw_visualization_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Visualization")
        .border_style(panel_style(app, Panel::Visualization));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Different layout depending on whether spectrum analyzer is shown
    if app.config.show_spectrum_analyzer {
        // Layout with spectrum analyzer at bottom
        let viz_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // Audio amplitude levels (L/R)
                Constraint::Length(2),  // Detected frequency bars (L/R)
                Constraint::Length(1),  // Spacer
                Constraint::Length(2),  // Output values A
                Constraint::Length(2),  // Output values B
                Constraint::Min(3),     // Spectrum analyzer (fills remaining)
            ])
            .split(inner_area);

        draw_amplitude_meters(frame, app, viz_chunks[0]);
        draw_frequency_meters(frame, app, viz_chunks[1]);
        // viz_chunks[2] is spacer
        draw_output_values(frame, app, viz_chunks[3], viz_chunks[4]);
        draw_spectrum_analyzer(frame, app, viz_chunks[5]);
    } else {
        // Original layout with bass/mid/treble
        let viz_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // Audio amplitude levels (L/R)
                Constraint::Length(2),  // Detected frequency bars (L/R)
                Constraint::Length(3),  // Frequency bands (bass/mid/treble)
                Constraint::Length(1),  // Spacer
                Constraint::Length(2),  // Output values A
                Constraint::Length(2),  // Output values B
                Constraint::Min(0),     // Remaining space
            ])
            .split(inner_area);

        draw_amplitude_meters(frame, app, viz_chunks[0]);
        draw_frequency_meters(frame, app, viz_chunks[1]);
        draw_frequency_spectrum(frame, app, viz_chunks[2]);
        // viz_chunks[3] is spacer
        draw_output_values(frame, app, viz_chunks[4], viz_chunks[5]);
    }
}

fn draw_amplitude_meters(frame: &mut Frame, app: &App, area: Rect) {
    // Split into label row and meter row
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Label row
    let label = Line::from(vec![
        Span::styled("Amplitude ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("L:{:>3}%", (app.audio_levels.0 * 100.0) as u8),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("  "),
        Span::styled(
            format!("R:{:>3}%", (app.audio_levels.1 * 100.0) as u8),
            Style::default().fg(Color::Magenta),
        ),
    ]);
    frame.render_widget(Paragraph::new(label), rows[0]);

    // Meter row
    let meter_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Percentage(45),
            Constraint::Length(2),
            Constraint::Percentage(45),
            Constraint::Length(2),
        ])
        .split(rows[1]);

    // Left label
    frame.render_widget(
        Paragraph::new("L ").style(Style::default().fg(Color::Cyan)),
        meter_chunks[0],
    );

    // Left meter - green/yellow/red based on level
    let left_ratio = app.audio_levels.0 as f64;
    let left_color = level_color(app.audio_levels.0);
    let left_gauge = Gauge::default()
        .gauge_style(Style::default().fg(left_color))
        .ratio(left_ratio.min(1.0));
    frame.render_widget(left_gauge, meter_chunks[1]);

    // Center spacer
    frame.render_widget(
        Paragraph::new("  ").style(Style::default().fg(Color::DarkGray)),
        meter_chunks[2],
    );

    // Right meter
    let right_ratio = app.audio_levels.1 as f64;
    let right_color = level_color(app.audio_levels.1);
    let right_gauge = Gauge::default()
        .gauge_style(Style::default().fg(right_color))
        .ratio(right_ratio.min(1.0));
    frame.render_widget(right_gauge, meter_chunks[3]);

    // Right label
    frame.render_widget(
        Paragraph::new(" R").style(Style::default().fg(Color::Magenta)),
        meter_chunks[4],
    );
}

fn level_color(level: f32) -> Color {
    if level > 0.9 {
        Color::Red
    } else if level > 0.7 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn draw_frequency_meters(frame: &mut Frame, app: &App, area: Rect) {
    let min_hz = app.config.freq_band_min;
    let max_hz = app.config.freq_band_max;
    let detected_left = app.output_values.detected_frequency_left;
    let detected_right = app.output_values.detected_frequency_right;

    // Split into label row and meter row
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Format frequency display strings
    let left_hz_str = match detected_left {
        Some(freq) => format!("{:>4.0}Hz", freq),
        None => "  --Hz".to_string(),
    };
    let right_hz_str = match detected_right {
        Some(freq) => format!("{:>4.0}Hz", freq),
        None => "  --Hz".to_string(),
    };

    // Label row: "Frequency  L:xxxHz  R:xxxHz  [min-max]"
    let label = Line::from(vec![
        Span::styled("Frequency  ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("L:", Style::default().fg(Color::Cyan)),
        Span::styled(left_hz_str, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("R:", Style::default().fg(Color::Magenta)),
        Span::styled(right_hz_str, Style::default().fg(Color::Magenta)),
        Span::raw("  "),
        Span::styled(
            format!("[{:.0}-{:.0}]", min_hz, max_hz),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(label), rows[0]);

    // Meter row: two horizontal bars showing where detected freq falls in the configured band
    let meter_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Percentage(45),
            Constraint::Length(2),
            Constraint::Percentage(45),
            Constraint::Length(2),
        ])
        .split(rows[1]);

    // Left label
    frame.render_widget(
        Paragraph::new("L ").style(Style::default().fg(Color::Cyan)),
        meter_chunks[0],
    );

    // Left frequency position gauge
    let left_ratio = detected_left
        .map(|freq| ((freq - min_hz) / (max_hz - min_hz)).clamp(0.0, 1.0) as f64)
        .unwrap_or(0.0);
    let left_gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(left_ratio);
    frame.render_widget(left_gauge, meter_chunks[1]);

    // Center spacer
    frame.render_widget(
        Paragraph::new("  ").style(Style::default().fg(Color::DarkGray)),
        meter_chunks[2],
    );

    // Right frequency position gauge
    let right_ratio = detected_right
        .map(|freq| ((freq - min_hz) / (max_hz - min_hz)).clamp(0.0, 1.0) as f64)
        .unwrap_or(0.0);
    let right_gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Magenta))
        .ratio(right_ratio);
    frame.render_widget(right_gauge, meter_chunks[3]);

    // Right label
    frame.render_widget(
        Paragraph::new(" R").style(Style::default().fg(Color::Magenta)),
        meter_chunks[4],
    );
}

fn draw_frequency_spectrum(frame: &mut Frame, app: &App, area: Rect) {
    // Split into 3 columns for bass, mid, treble
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(area);

    let bands = &app.frequency_bands;

    // Bass (20-250 Hz) - Blue/Cyan
    draw_frequency_bar(frame, chunks[0], "Bass", bands.bass, Color::Blue);

    // Mid (250-4000 Hz) - Green
    draw_frequency_bar(frame, chunks[1], "Mid", bands.mid, Color::Green);

    // Treble (4000-20000 Hz) - Magenta
    draw_frequency_bar(frame, chunks[2], "Treble", bands.treble, Color::Magenta);
}

fn draw_frequency_bar(frame: &mut Frame, area: Rect, label: &str, value: f32, color: Color) {
    // Split vertically: label on top, bar below
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Label with value
    let label_text = format!("{}: {:.0}%", label, value * 100.0);
    frame.render_widget(
        Paragraph::new(label_text).style(Style::default().fg(color)),
        chunks[0],
    );

    // Horizontal bar using block chars
    let bar_width = chunks[1].width.saturating_sub(1) as usize;
    let filled = ((value * bar_width as f32) as usize).min(bar_width);
    let bar: String = (0..bar_width)
        .map(|i| if i < filled { '#' } else { '-' })
        .collect();

    frame.render_widget(
        Paragraph::new(bar).style(Style::default().fg(color)),
        chunks[1],
    );
}

/// Draw spectrum analyzer with vertical bars - split left/right channels
fn draw_spectrum_analyzer(frame: &mut Frame, app: &App, area: Rect) {
    if area.height < 2 || area.width < 8 {
        return;
    }

    // Split area into left and right halves with a small gap
    let half_width = (area.width / 2).saturating_sub(1);
    let left_area = Rect::new(area.x, area.y, half_width, area.height);
    let right_area = Rect::new(area.x + half_width + 2, area.y, half_width, area.height);

    // Draw left channel (cyan tint)
    draw_spectrum_half(frame, &app.spectrum_left, left_area, Color::Cyan, "L");

    // Draw separator
    for row in 0..area.height {
        let sep_area = Rect::new(area.x + half_width, area.y + row, 2, 1);
        frame.render_widget(
            Paragraph::new("|").style(Style::default().fg(Color::DarkGray)),
            sep_area,
        );
    }

    // Draw right channel (magenta tint)
    draw_spectrum_half(frame, &app.spectrum_right, right_area, Color::Magenta, "R");
}

/// Draw one half of the spectrum analyzer (single channel)
fn draw_spectrum_half(
    frame: &mut Frame,
    spectrum: &[f32; SPECTRUM_BARS],
    area: Rect,
    base_color: Color,
    _label: &str,
) {
    if area.height < 1 || area.width < 2 {
        return;
    }

    // Calculate how many bars we can fit
    let available_width = area.width as usize;
    let num_bars = SPECTRUM_BARS.min(available_width);

    // Calculate bar width and spacing
    let bar_width = 1;
    let total_bar_width = num_bars * bar_width;
    let spacing = if available_width > total_bar_width {
        (available_width - total_bar_width) / num_bars.max(1)
    } else {
        0
    };
    let cell_width = bar_width + spacing;

    // Block characters for vertical bars (from empty to full)
    let bar_chars = [' ', '_', '.', '-', '=', '#', '@'];
    let max_height = area.height as usize;

    // Build the display line by line (top to bottom)
    let mut lines = Vec::with_capacity(max_height);

    for row in 0..max_height {
        let threshold = 1.0 - (row as f32 / max_height as f32);
        let mut line = String::with_capacity(available_width);

        for bar_idx in 0..num_bars {
            // Map bar index to spectrum data
            let spectrum_idx = (bar_idx * SPECTRUM_BARS) / num_bars;
            let value = spectrum.get(spectrum_idx).copied().unwrap_or(0.0);

            // Determine if this cell should be filled
            let char_to_use = if value >= threshold {
                '#'
            } else if value >= threshold - (1.0 / max_height as f32) {
                let fraction = (value - (threshold - 1.0 / max_height as f32)) * max_height as f32;
                let idx = ((fraction * (bar_chars.len() - 1) as f32) as usize).min(bar_chars.len() - 1);
                bar_chars[idx]
            } else {
                ' '
            };

            line.push(char_to_use);

            // Add spacing
            for _ in 0..spacing {
                line.push(' ');
            }
        }

        lines.push(line);
    }

    // Render each line with color gradient based on frequency band
    for (row, line_text) in lines.iter().enumerate() {
        let y = area.y + row as u16;
        if y >= area.y + area.height {
            break;
        }

        let mut spans = Vec::new();
        let chars: Vec<char> = line_text.chars().collect();
        let num_visible_bars = num_bars.min(chars.len());

        for (i, &ch) in chars.iter().enumerate() {
            let bar_idx = if cell_width > 0 { i / cell_width } else { i };

            // Color gradient: low freq = base color, mid = green, high = varies
            let color = if bar_idx < num_visible_bars / 3 {
                base_color
            } else if bar_idx < 2 * num_visible_bars / 3 {
                Color::Green
            } else {
                Color::Yellow
            };

            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(if ch == ' ' { Color::DarkGray } else { color }),
            ));
        }

        let line_widget = Paragraph::new(Line::from(spans));
        let line_area = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(line_widget, line_area);
    }
}

fn draw_output_values(frame: &mut Frame, app: &App, area_a: Rect, area_b: Rect) {
    let output = &app.output_values;

    // Channel A - intensity and output Hz
    // coyote_freq is period in ms, so output Hz = 1000 / coyote_freq
    let output_hz_a = if output.coyote_frequency_a > 0 {
        1000 / output.coyote_frequency_a
    } else {
        0
    };
    let a_lines = vec![
        Line::from(vec![
            Span::styled(
                "Channel A ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Intensity: "),
            Span::styled(
                format!("{:>4}", output.intensity_a),
                Style::default().fg(Color::White),
            ),
            Span::raw("  Out: "),
            Span::styled(
                format!("{:>3}Hz", output_hz_a),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(a_lines), area_a);

    // Channel B - intensity and output Hz
    let output_hz_b = if output.coyote_frequency_b > 0 {
        1000 / output.coyote_frequency_b
    } else {
        0
    };
    let b_lines = vec![
        Line::from(vec![
            Span::styled(
                "Channel B ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Intensity: "),
            Span::styled(
                format!("{:>4}", output.intensity_b),
                Style::default().fg(Color::White),
            ),
            Span::raw("  Out: "),
            Span::styled(
                format!("{:>3}Hz", output_hz_b),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(b_lines), area_b);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),  // Connection state + pause indicator
            Constraint::Percentage(45),  // Status/Error message
            Constraint::Percentage(35),  // Help/key hints
        ])
        .split(area);

    // Connection state with pause indicator
    let conn_block = Block::default().borders(Borders::ALL);
    let (conn_text, conn_color) = match app.connection_state {
        ConnectionState::Disconnected => ("Disconnected".to_string(), Color::Red),
        ConnectionState::Connecting => ("Connecting...".to_string(), Color::Yellow),
        ConnectionState::Connected => {
            if let Some(version) = app.connected_device_version {
                (format!("Connected ({})", version), Color::Green)
            } else {
                ("Connected".to_string(), Color::Green)
            }
        }
        ConnectionState::Reconnecting => ("Reconnecting...".to_string(), Color::Yellow),
    };

    // Build spans for connection + pause indicator
    let mut conn_spans = vec![Span::styled(conn_text, Style::default().fg(conn_color))];
    if app.is_paused {
        conn_spans.push(Span::styled(
            " [PAUSED]",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    let conn_line = Line::from(conn_spans);
    let conn_paragraph = Paragraph::new(conn_line).block(conn_block);
    frame.render_widget(conn_paragraph, chunks[0]);

    // Status/Error message
    let msg_block = Block::default().borders(Borders::ALL);
    let (msg_text, msg_color) = if let Some(ref err) = app.error_message {
        (err.as_str(), Color::Red)
    } else if let Some(ref status) = app.status_message {
        (status.as_str(), Color::White)
    } else {
        ("", Color::White)
    };
    let msg_paragraph = Paragraph::new(msg_text)
        .style(Style::default().fg(msg_color))
        .block(msg_block);
    frame.render_widget(msg_paragraph, chunks[1]);

    // Key hints
    let help_block = Block::default().borders(Borders::ALL);
    let help_paragraph = Paragraph::new("q:Quit p:Pause a:Analyzer ?:Help")
        .style(Style::default().fg(Color::DarkGray))
        .block(help_block);
    frame.render_widget(help_paragraph, chunks[2]);
}

/// Create a centered rectangle with the given percentage of width and height
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

/// Build the help text content
fn build_help_content() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "COYOTE AUDIO",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Audio-reactive controller for Coyote e-stim devices."),
        Line::from("Routes left/right audio channels to channels A/B."),
        Line::from(""),
        Line::from(Span::styled(
            "KEYBINDINGS",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  q       ", Style::default().fg(Color::Yellow)),
            Span::raw("Quit application"),
        ]),
        Line::from(vec![
            Span::styled("  p       ", Style::default().fg(Color::Yellow)),
            Span::raw("Pause/unpause output"),
        ]),
        Line::from(vec![
            Span::styled("  a       ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle spectrum analyzer"),
        ]),
        Line::from(vec![
            Span::styled("  Esc     ", Style::default().fg(Color::Yellow)),
            Span::raw("Emergency stop (zero output)"),
        ]),
        Line::from(vec![
            Span::styled("  ?       ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle this help screen"),
        ]),
        Line::from(vec![
            Span::styled("  Tab     ", Style::default().fg(Color::Yellow)),
            Span::raw("Cycle through panels"),
        ]),
        Line::from(vec![
            Span::styled("  Up/Down ", Style::default().fg(Color::Yellow)),
            Span::raw("Navigate items / scroll help"),
        ]),
        Line::from(vec![
            Span::styled("  Left/Right ", Style::default().fg(Color::Yellow)),
            Span::raw("Adjust selected value"),
        ]),
        Line::from(vec![
            Span::styled("  Enter   ", Style::default().fg(Color::Yellow)),
            Span::raw("Select / connect to device"),
        ]),
        Line::from(vec![
            Span::styled("  [ / ]   ", Style::default().fg(Color::Yellow)),
            Span::raw("Large adjustment (10x step)"),
        ]),
        Line::from(vec![
            Span::styled("  s       ", Style::default().fg(Color::Yellow)),
            Span::raw("Scan for Coyote devices"),
        ]),
        Line::from(vec![
            Span::styled("  d       ", Style::default().fg(Color::Yellow)),
            Span::raw("Disconnect from device"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "SETTINGS",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Max Intensity A/B ", Style::default().fg(Color::Green)),
            Span::raw("Caps output level (0-100%)"),
        ]),
        Line::from(vec![
            Span::styled("  Freq Min/Max      ", Style::default().fg(Color::Green)),
            Span::raw("Audio frequency range for mapping"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "HOW IT WORKS",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Audio is captured from the configured PipeWire source."),
        Line::from("  Use pavucontrol's Recording tab to select"),
        Line::from("  \"Monitor of {device}\" where your audio plays."),
        Line::from(""),
        Line::from("  Left channel controls Channel A, right controls Channel B."),
        Line::from(""),
        Line::from("  For each channel:"),
        Line::from("    - Amplitude (volume) -> Output intensity"),
        Line::from("    - Detected frequency -> Coyote pulse rate (10-100)"),
        Line::from(""),
        Line::from("  The frequency band settings define which audio frequencies"),
        Line::from("  are analyzed and mapped to the Coyote frequency range."),
        Line::from(""),
        Line::from(Span::styled(
            "COYOTE V3 PAIRING (firmware v10+)",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  If a V3 device doesn't appear when scanning, it may"),
        Line::from("  need to be put into pairing mode after a firmware update."),
        Line::from(""),
        Line::from("  Pull down (not press) BOTH wheels simultaneously and"),
        Line::from("  hold until the shoulder lights flash yellow and blue,"),
        Line::from("  then release. The device will now appear in scans."),
        Line::from(""),
        Line::from("  Once bonded, this only needs to be done again if you"),
        Line::from("  switch computers or clear Bluetooth pairings."),
        Line::from("  V2 devices are not affected."),
        Line::from(""),
        Line::from(Span::styled(
            "---",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Use Up/Down or j/k to scroll. Press Esc or q to close.",
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

/// Draw the help modal overlay
fn draw_help_modal(frame: &mut Frame, app: &App) {
    // Create a centered rectangle (80% width, 70% height)
    let area = centered_rect(80, 70, frame.area());

    // Clear the area first to erase any underlying content
    frame.render_widget(Clear, area);

    // Build the help content
    let help_lines = build_help_content();
    let total_lines = help_lines.len() as u16;

    // Calculate max scroll offset (leave some visible lines)
    let inner_height = area.height.saturating_sub(2); // Account for borders
    let max_scroll = total_lines.saturating_sub(inner_height);
    let scroll_offset = app.help_scroll_offset.min(max_scroll);

    // Create the paragraph with scroll and solid background
    let help_text = Paragraph::new(help_lines)
        .style(Style::default().bg(Color::Black))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Help - Press Esc to close ")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(Color::Black)),
        )
        .scroll((scroll_offset, 0));

    frame.render_widget(help_text, area);

    // Draw scroll indicator if content is scrollable
    if total_lines > inner_height {
        let indicator = format!(" [{}/{}] ", scroll_offset + 1, max_scroll + 1);
        let indicator_span = Span::styled(indicator, Style::default().fg(Color::DarkGray));

        // Position in bottom-right of the modal border area
        let indicator_area = Rect::new(
            area.x + area.width.saturating_sub(indicator_span.width() as u16 + 2),
            area.y + area.height.saturating_sub(1),
            indicator_span.width() as u16,
            1,
        );
        frame.render_widget(Paragraph::new(indicator_span), indicator_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_color() {
        assert_eq!(level_color(0.0), Color::Green);
        assert_eq!(level_color(0.5), Color::Green);
        assert_eq!(level_color(0.75), Color::Yellow);
        assert_eq!(level_color(0.95), Color::Red);
    }
}
