# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Audio-reactive e-stim controller for DG-Lab Coyote V2/V3 devices. Captures audio via PipeWire, analyzes amplitude and frequency, and sends commands over BLE.

## Build & Test

```bash
cargo build --release    # Binary: target/release/coyote-audio
cargo test --lib         # Run unit tests (74 tests)
```

## Architecture

**Data flow:**
```
PipeWire Audio -> AudioAnalyzer -> AudioMapper -> BLE -> Coyote Device
     (L/R)         (RMS+FFT)      (curves/ramp)   (protocol)
```

**Concurrency model:**
- PipeWire audio capture runs in a dedicated OS thread (`std::thread`)
- Main event loop uses `tokio::select!` with three ticks: TUI (~30fps), BLE commands (100ms), and async events
- Shared state between threads protected by `Arc<Mutex<SharedState>>`

## Project Structure

```
src/
├── main.rs           # Async runtime, tokio::select! main loop
├── config.rs         # Settings persistence (~/.config/coyote-audio/)
├── audio/
│   ├── pipewire.rs   # PipeWire sink, audio capture thread
│   ├── analysis.rs   # RMS amplitude, FFT, frequency detection
│   └── mapper.rs     # Audio -> Coyote values, mapping curves, ramp-up
├── ble/
│   ├── scanner.rs    # Device discovery (V2: "D-LAB ESTIM01", V3: "47L121000")
│   ├── connection.rs # Connection management, reconnection logic
│   ├── protocol.rs   # Coyote V2 encoding (PWM_AB2, PWM_A34, PWM_B34)
│   └── protocol_v3.rs # Coyote V3 encoding (unified characteristic, seq numbers)
└── tui/
    ├── app.rs        # App state, event handling, key bindings
    └── ui.rs         # Widget rendering, spectrum analyzer
```

## Protocol Details

**Coyote V2:**
- Service UUID: `955a180B-0FE2-F5AA-A094-84B8D4F3E8AD`
- Characteristics: `0x1504` (intensity), `0x1505` (ch A waveform), `0x1506` (ch B waveform)
- Command interval: 100ms

**Coyote V3:**
- Single unified characteristic with sequence numbering
- Different byte encoding than V2

## Key Concepts

- **Stereo split**: Left audio -> Channel A, Right audio -> Channel B
- **Intensity range**: 0-2047 per channel
- **Mapping curves**: Linear, Exponential, Logarithmic, S-Curve
- **Safety**: Soft ramp-up (500ms), instant decrease, Esc = emergency stop

## Config

`~/.config/coyote-audio/config.toml` - auto-saved on quit

## Common Issues

- **BLE not finding device**: Ensure Bluetooth on, device powered, not connected elsewhere
- **PipeWire sink missing**: `systemctl --user status pipewire`
- **Terminal messed up**: Run `reset`
