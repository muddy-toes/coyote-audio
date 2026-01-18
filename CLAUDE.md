# Coyote Audio - Project Instructions

## Overview

Audio-reactive e-stim controller for DG-Lab Coyote V2 devices. Captures audio via PipeWire, analyzes amplitude and frequency, and sends commands over BLE to the Coyote.

## Tech Stack

- **Language**: Rust (2021 edition)
- **Audio**: PipeWire via `pipewire` crate
- **BLE**: `btleplug` crate
- **TUI**: `ratatui` with `crossterm` backend
- **FFT**: `rustfft` for frequency analysis
- **Config**: `serde` + `toml` for settings persistence

## Building

```bash
cargo build --release
# Binary: target/release/coyote-audio
```

## Project Structure

```
src/
├── main.rs           # Entry point, async runtime, main loop
├── lib.rs            # Library exports
├── config.rs         # Settings persistence (~/.config/coyote-audio/)
├── audio/
│   ├── mod.rs
│   ├── pipewire.rs   # PipeWire sink creation and audio capture
│   ├── analysis.rs   # RMS, FFT, frequency detection (per-channel)
│   └── mapper.rs     # Audio analysis -> Coyote protocol values
├── ble/
│   ├── mod.rs
│   ├── scanner.rs    # BLE device discovery
│   ├── connection.rs # Connection management, reconnection
│   └── protocol.rs   # Coyote V2 command encoding
└── tui/
    ├── mod.rs
    ├── app.rs        # App state, event handling, key bindings
    └── ui.rs         # Widget rendering, layouts, help modal
```

## Coyote V2 Protocol

- **Service UUID**: `955a180B-0FE2-F5AA-A094-84B8D4F3E8AD`
- **Characteristics**:
  - `0x1504` (PWM_AB2): Intensity for both channels (0-2047 each)
  - `0x1505` (PWM_A34): Channel A waveform (X, Y, Z params)
  - `0x1506` (PWM_B34): Channel B waveform (X, Y, Z params)
- **Timing**: Commands must be resent every 100ms

## Key Bindings

- `q`: Quit (fully exit, restore terminal)
- `p`: Pause/unpause output
- `Esc`: Emergency stop (zero all output)
- `?`: Help modal (scroll with arrows, close with Esc)
- `Tab`: Cycle panels
- Arrows: Navigate/adjust values
- `Enter`: Select/connect

## Audio Processing

- **Stereo split**: Left audio -> Channel A, Right audio -> Channel B
- **Amplitude**: Always controls intensity (0-2047 range)
- **Frequency**: Configurable band (min/max Hz) maps to Coyote frequency (10-1000 Hz)
- **Mapping curves**: Linear, Exponential, Logarithmic

## Config Location

`~/.config/coyote-audio/config.toml`

## Testing

Run the app and verify:
1. PipeWire sink appears in pavucontrol
2. BLE scanning finds "D-LAB ESTIM01" devices
3. Audio visualization responds to sound
4. Output correlates with audio when connected

## Common Issues

- **BLE not finding device**: Ensure Bluetooth is on, device is powered, not connected elsewhere
- **PipeWire sink missing**: Check PipeWire is running (`systemctl --user status pipewire`)
- **Terminal messed up after crash**: Run `reset` command
