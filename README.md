# Coyote Audio

Audio-reactive e-stim controller for the DG-Lab Coyote device and linux.

Captures system audio via PipeWire, analyzes amplitude and frequency, and sends real-time stimulation commands over Bluetooth Low Energy.

THIS IS EXPERIMENTAL SOFTWARE.  USE AT YOUR OWN RISK.

In particular, Coyote V3 support has not even been tested as I do not own a V3 device.

## How It Works

```
PipeWire Audio -> FFT Analysis -> Amplitude/Frequency Mapping -> BLE Commands -> Coyote V2
```

- **Stereo separation**: Left channel controls Channel A, right channel controls Channel B
- **Amplitude** (loudness) maps to stimulation intensity
- **Detected frequency** (dominant pitch) maps to the Coyote's internal frequency parameter

## Requirements

- **Rust toolchain** (1.70+)
- **PipeWire** with development libraries (`pipewire-devel` or `libpipewire-0.3-dev`)
- **BlueZ** (Linux Bluetooth stack)
- **DG-Lab Coyote V2** device

### Debian/Ubuntu

```bash
sudo apt install libpipewire-0.3-dev libbluetooth-dev libdbus-1-dev
```

### Fedora

```bash
sudo dnf install pipewire-devel bluez-libs-devel dbus-devel
```

## Building

```bash
cargo build --release
```

Binary will be at `target/release/coyote-audio`.

## Usage

```bash
./target/release/coyote-audio
```

### Basic Workflow

1. Launch the application
2. Press `s` to scan for Coyote devices
3. Use arrow keys to select your device
4. Press `Enter` to connect
5. Play audio - the app automatically creates a PipeWire sink that captures system audio
6. Adjust parameters as needed
7. Press `q` to quit (config is saved automatically)

## Controls

### Audio device

This works by listening to an existing audio device.  If you run `pavucontrol` and look at the Recording tab while coyote-audio is running,
that's where you'll see it listed.  Select "Monitor of {device name}" to choose which device your e-stim audio source is connected to.

### Global

| Key | Action |
|-----|--------|
| `q` / `Q` | Quit (saves config) |
| `Ctrl+C` | Quit immediately |
| `Tab` | Next panel |
| `Shift+Tab` | Previous panel |

### Devices Panel

| Key | Action |
|-----|--------|
| `s` | Scan for devices |
| `Up` / `k` | Select previous device |
| `Down` / `j` | Select next device |
| `Enter` / `Space` | Connect/disconnect |
| `d` | Disconnect |

### Frequency Band / Parameters Panels

| Key | Action |
|-----|--------|
| `Up` / `k` | Previous parameter |
| `Down` / `j` | Next parameter |
| `Left` / `h` | Decrease value |
| `Right` / `l` | Increase value |
| `[` | Decrease by 10x |
| `]` | Increase by 10x |

## Configuration

Config file: `~/.config/coyote-audio/config.toml`

```toml
max_intensity_a = 1024    # Channel A max (0-2047)
max_intensity_b = 1024    # Channel B max (0-2047)
sensitivity = 0.5         # Audio sensitivity (0.0-1.0)
freq_band_min = 200.0     # Min Hz for frequency mapping
freq_band_max = 800.0     # Max Hz for frequency mapping
last_device_address = "XX:XX:XX:XX:XX:XX"  # Auto-saved
```

### Settings Explained

- **Max Intensity A/B**: Caps the maximum output intensity per channel. Default 1024 is 50% of the device's maximum.
- **Sensitivity**: Gain control for how strongly audio amplitude drives stimulation intensity (see below).
- **Freq Band Min/Max**: Audio frequencies within this range are mapped to Coyote frequency output. Frequencies outside this range use a default.

### Sensitivity

The sensitivity slider (0.0-1.0, default 0.5) acts as a gain control between audio amplitude and stimulation intensity.

**Formula**: `scaled = (amplitude * sensitivity * 2.0).clamp(0.0, 1.0)`

| Sensitivity | Effect |
|-------------|--------|
| 0.5 (default) | Full-scale audio (1.0) produces full intensity |
| 1.0 | Half-amplitude audio (0.5) produces full intensity |
| 0.25 | Even full-scale audio only reaches 50% intensity |

Think of it as a volume knob for how strongly audio drives stimulation:
- **Higher values** = quieter audio produces stronger output (more responsive)
- **Lower values** = need louder audio to reach the same intensity (more headroom)

## Audio Processing

### Amplitude to Intensity

Audio amplitude (RMS level) is converted to stimulation intensity:

1. RMS calculated per channel (L/R separate)
2. Scaled by sensitivity setting
3. Mapping curve applied (see below)
4. Capped at max intensity setting
5. Soft ramp-up applied to prevent sudden jumps

### Frequency to Coyote Frequency

The dominant frequency detected via FFT is mapped to the Coyote's frequency parameter:

1. FFT performed per channel (L/R separate)
2. Dominant frequency identified (highest magnitude bin)
3. If within configured band (default 200-800 Hz), linearly mapped to Coyote's 10-1000 range
4. Frequencies outside the band use a default of 500

**Example**: With default settings (200-800 Hz band):
- 200 Hz audio -> Coyote freq 10
- 500 Hz audio -> Coyote freq 505
- 800 Hz audio -> Coyote freq 1000

### Stereo Routing

- **Left audio channel** -> Channel A intensity + frequency
- **Right audio channel** -> Channel B intensity + frequency

This allows stereo-aware content to produce different sensations per channel.

## Mapping Curves

Control how audio amplitude translates to intensity:

| Curve | Formula | Behavior |
|-------|---------|----------|
| **Linear** | `output = input` | Direct 1:1 mapping |
| **Exponential** | `output = input^2` | More responsive at low levels, compressed at high |
| **Logarithmic** | `output = ln(1 + input*(e-1))` | More responsive at high levels, gentle at low |
| **S-Curve** | `output = 3x^2 - 2x^3` | Gentle at extremes, steeper in middle |

Choose based on your audio source:
- **Exponential**: Good for music with wide dynamic range
- **Logarithmic**: Good for speech or quiet audio
- **S-Curve**: Balanced, natural feel
- **Linear**: Direct control, predictable

## Safety

### Soft Ramp-Up

On connection, intensity ramps up gradually over 500ms rather than jumping immediately. This prevents unexpected strong sensations.

### Instant Decrease

While increases are ramped, intensity can decrease instantly when audio gets quieter.

### If Something Goes Wrong

- **Close the app** (`q` or `Ctrl+C`) - disconnects and stops output
- **Emergency Stop** Press Esc to immediately set output levels to zero
- **Pause Signal** Press 'p' to pause/unpause output

## Troubleshooting

### BLE Issues

**"BLE init failed"**
- Ensure Bluetooth is enabled: `bluetoothctl power on`
- Check BlueZ is running: `systemctl status bluetooth`
- May need root or `bluetooth` group membership

**No devices found**
- Ensure Coyote is powered on and not connected to another device
- Try `bluetoothctl scan on` to verify your adapter sees it
- Device advertises as "D-LAB ESTIM01" or similar

**Connection drops frequently**
- Move closer to reduce interference
- App will attempt automatic reconnection

### PipeWire Issues

**"Audio init failed"**
- Ensure PipeWire is running: `systemctl --user status pipewire`
- Check development libraries are installed

**No audio being captured**
- The app creates a virtual sink - route audio to it via `pavucontrol` or `pw-top`
- Check that your audio player is outputting to the correct device

**Low latency needed**
- PipeWire typically has lower latency than PulseAudio
- Adjust quantum settings in PipeWire config if needed

## Technical Details

- Protocol: DG-Lab Coyote V2 BLE (PWM_AB2, PWM_A34, PWM_B34 characteristics)
- Sample rate: 48kHz stereo (PipeWire default)
- Analysis window: ~1024 samples
- BLE command rate: ~30Hz
- TUI refresh: ~30 FPS

## License

MIT
