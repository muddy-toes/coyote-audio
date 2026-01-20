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

**FFT optimization:**
- Single FFT pass per channel extracts all frequency data (dominant freq, bands, spectrum)
- 2 FFTs per frame total (one per stereo channel)
- 8192-point FFT with Hann window for ~5.9 Hz resolution at 48kHz

## Project Structure

```
src/
├── main.rs           # Async runtime, tokio::select! main loop
├── config.rs         # Settings persistence (~/.config/coyote-audio/)
├── audio/
│   ├── pipewire.rs   # PipeWire sink, audio capture thread
│   ├── analysis.rs   # RMS amplitude, FFT, frequency detection (FftResult struct)
│   └── mapper.rs     # Audio -> Coyote values, mapping curves, ramp-up
├── ble/
│   ├── scanner.rs    # Device discovery (V2: "D-LAB ESTIM01", V3: "47L121000")
│   ├── connection.rs # Connection management, reconnection, V2/V3 initialization
│   ├── protocol.rs   # Coyote V2 encoding (PWM_AB2, PWM_A34, PWM_B34)
│   └── protocol_v3.rs # Coyote V3 encoding (B0 command, BF soft limits)
└── tui/
    ├── app.rs        # App state, event handling, key bindings
    └── ui.rs         # Widget rendering, spectrum analyzer
```

## Protocol Details

**Always** defer to the protocol docs, with the exception of the channel swap part.  Our channels are not swapped.

- "Humans perceive frequency changes slowly - if you change frequency every 100ms it feels mushy"
- "Humans perceive pulse width changes quickly - Z modulation every 100ms creates distinct, varied sensations"

### Coyote V2

- **Service UUID:** `955a180B-0FE2-F5AA-A094-84B8D4F3E8AD`
- **Characteristics:**
  - `0x1504` (PWM_AB2): Combined intensity for both channels
  - `0x1505` (PWM_A34): Channel A waveform
  - `0x1506` (PWM_B34): Channel B waveform
- **Intensity range:** 0-2047 per channel (11 bits each, packed into 3 bytes)
- **Frequency range:** 10-100 (X+Y waveform parameters)
- **Command interval:** 100ms

**Important - Channel naming quirk:** The official DG-LAB docs show PWM_A34 as "B通道波形数据" (B channel waveform) and PWM_B34 as "A通道波形数据" (A channel waveform), which is counterintuitive. However, our implementation sends channel A waveform to PWM_A34 and channel B to PWM_B34, and this works correctly in practice. The docs appear to have the names swapped or there's a translation issue. **Do not change the channel mapping** - it has been verified to work as implemented.

### Coyote V3

- **Service UUID:** `0x180C` (standard BLE base)
- **Write characteristic:** `0x150A`
- **Notify characteristic:** `0x150B`
- **Battery:** `0x180A` / `0x1500`

**B0 Command (20 bytes, sent every 100ms):**
```
[0]     = 0xB0 (command header)
[1]     = sequence (4 bits) | intensity_mode (4 bits)
[2]     = Channel A intensity (0-200)
[3]     = Channel B intensity (0-200)
[4-7]   = Channel A frequencies (4 x 25ms segments, 10-240 each)
[8-11]  = Channel A waveform intensities (4 segments, 0-100 each)
[12-15] = Channel B frequencies
[16-19] = Channel B waveform intensities
```

**BF Command (7 bytes, sent on connect):**
```
[0] = 0xBF (command header)
[1] = Channel A soft limit (0-200)
[2] = Channel B soft limit (0-200)
[3] = Channel A frequency balance (0-255)
[4] = Channel B frequency balance (0-255)
[5] = Channel A intensity balance (0-255)
[6] = Channel B intensity balance (0-255)
```

The BF command MUST be sent after every connect/reconnect to set soft limits. We send soft limits of 200 (max) and balance params of 128 (neutral).

**Frequency mapping:** The mapper outputs frequencies in the V2 range (10-100). For V3 devices, `protocol_v3.rs` scales this to the full V3 range (10-240) using linear interpolation.

**Intensity mapping:** V2 uses 0-2047, V3 uses 0-200. The protocol handles conversion.

## Key Concepts

- **Stereo split:** Left audio -> Channel A, Right audio -> Channel B
- **Intensity range:** 0-2047 internally (V2 native), scaled to 0-200 for V3
- **Frequency range:** 10-100 from mapper, scaled to 10-240 for V3
- **Mapping curves:** Linear, Exponential, Logarithmic, S-Curve
- **Safety:** Soft ramp-up (100ms), instant decrease, Esc = emergency stop

## Config

`~/.config/coyote-audio/config.toml` - auto-saved on quit

Settings include:
- `max_intensity_a`, `max_intensity_b`: Per-channel intensity caps
- `sensitivity`: Audio input scaling
- `mapping_curve`: Intensity response curve
- `freq_band_min`, `freq_band_max`: Audio frequency range to map
- `show_spectrum_analyzer`: Toggle spectrum display
- `last_device_address`: For auto-reconnect

## Common Issues

- **BLE not finding device:** Ensure Bluetooth on, device powered, not connected elsewhere
- **PipeWire sink missing:** `systemctl --user status pipewire`
- **Terminal messed up:** Run `reset`
- **V3 device not responding:** BF command may have failed - reconnect

## Reference Documentation

The `references/DG-LAB-OPENSOURCE/` directory contains the official protocol documentation:
- `coyote/v2/README_V2.md` - V2 protocol spec
- `coyote/v3/README_V3.md` - V3 protocol spec
- `coyote/v3/example.md` - V3 usage examples
