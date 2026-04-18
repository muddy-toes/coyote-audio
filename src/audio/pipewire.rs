//! PipeWire audio capture module
//!
//! Creates a virtual sink that appears in pavucontrol, capturing audio
//! while passing it through to the default output.

use std::convert::TryInto;
use std::io::Cursor;
use std::mem;
use std::sync::{mpsc, Arc};
use std::thread;

use pipewire as pw;
use pipewire::main_loop::MainLoopBox;
use pipewire::stream::StreamBox;
use pw::properties::properties;
use pw::spa;
use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Pod, Value};
use pw::stream::{StreamFlags};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioCaptureError {
    #[error("Failed to initialize PipeWire: {0}")]
    InitError(String),
    #[error("Failed to create stream: {0}")]
    StreamError(String),
    #[error("PipeWire disconnected")]
    Disconnected,
    #[error("Channel send error")]
    ChannelError,
}

#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    pub sample_rate: u32,
    pub channels: u32,
    pub sink_name: String,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            sink_name: "Coyote Audio Capture".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub channels: u32,
    pub sample_rate: u32,
}

struct StreamUserData {
    audio_tx: mpsc::Sender<AudioBuffer>,
    format: AudioInfoRaw,
}

pub struct AudioCapture {
    config: AudioCaptureConfig,
    stop_sender: Option<mpsc::Sender<()>>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl AudioCapture {
    pub fn new(config: AudioCaptureConfig) -> Self {
        Self {
            config,
            stop_sender: None,
            thread_handle: None,
        }
    }

    pub fn start(&mut self) -> Result<mpsc::Receiver<AudioBuffer>, AudioCaptureError> {
        let (audio_tx, audio_rx) = mpsc::channel::<AudioBuffer>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let config = self.config.clone();

        let handle = thread::spawn(move || {
            if let Err(e) = run_pipewire_loop(config, audio_tx, stop_rx) {
                log::error!("PipeWire loop error: {}", e);
            }
        });

        self.stop_sender = Some(stop_tx);
        self.thread_handle = Some(handle);

        Ok(audio_rx)
    }

    pub fn stop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_pipewire_loop(
    config: AudioCaptureConfig,
    audio_tx: mpsc::Sender<AudioBuffer>,
    stop_rx: mpsc::Receiver<()>,
) -> Result<(), AudioCaptureError> {
    pw::init();

    let main_loop = Arc::new(MainLoopBox::new(None).map_err(|e| {
        AudioCaptureError::InitError(format!("Failed to create main loop: {}", e))
    })?);

    // Set up a timer to periodically check for stop signal
    // The timer must be kept alive (not dropped) for it to fire
    let loop_clone = main_loop.clone();
    let _stop_timer = main_loop.loop_().add_timer(move |_| {
        // Check if stop signal received (non-blocking)
        if stop_rx.try_recv().is_ok() {
            log::info!("Stop signal received, quitting PipeWire loop");
            loop_clone.quit();
        }
    });
    // Check every 100ms
    _stop_timer.update_timer(
        Some(std::time::Duration::from_millis(100)),
        Some(std::time::Duration::from_millis(100)),
    );

    let context = pw::context::ContextBox::new(&main_loop.loop_(), None).map_err(|e| {
        AudioCaptureError::InitError(format!("Failed to create context: {}", e))
    })?;

    let core = context.connect(None).map_err(|e| {
        AudioCaptureError::InitError(format!("Failed to connect to PipeWire: {}", e))
    })?;

    // Create stream with properties that make it appear as a virtual sink
    // that monitors audio (captures while passing through to default output)
    let stream = StreamBox::new(
        &core,
        &config.sink_name,
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::NODE_NAME => config.sink_name.as_str(),
            *pw::keys::NODE_DESCRIPTION => "Coyote Audio Capture - routes audio to analysis",
            // Stream capture sink mode: captures from a sink's monitor
            // This allows the audio to pass through while we capture it
            *pw::keys::STREAM_CAPTURE_SINK => "true",
        },
    )
    .map_err(|e| AudioCaptureError::StreamError(format!("Failed to create stream: {}", e)))?;

    let user_data = StreamUserData {
        audio_tx,
        format: AudioInfoRaw::default(),
    };

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_, _user_data, old, new| {
            log::info!("Stream state changed: {:?} -> {:?}", old, new);
        })
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }

            // Try to parse audio format info
            if user_data.format.parse(param).is_ok() {
                log::info!(
                    "Audio format: rate={} channels={}",
                    user_data.format.rate(),
                    user_data.format.channels()
                );
            }
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };

            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }

            let data = &mut datas[0];
            let n_channels = user_data.format.channels();
            let sample_rate = user_data.format.rate();
            let n_samples = data.chunk().size() / (mem::size_of::<f32>() as u32);

            if let Some(raw_samples) = data.data() {
                let samples: Vec<f32> = (0..n_samples)
                    .map(|n| {
                        let start = n as usize * mem::size_of::<f32>();
                        let end = start + mem::size_of::<f32>();
                        let bytes = &raw_samples[start..end];
                        f32::from_le_bytes(bytes.try_into().unwrap())
                    })
                    .collect();

                if !samples.is_empty() {
                    let audio_buffer = AudioBuffer {
                        samples,
                        channels: n_channels,
                        sample_rate,
                    };

                    if user_data.audio_tx.send(audio_buffer).is_err() {
                        log::warn!("Audio channel receiver dropped");
                    }
                }
            }
        })
        .register()
        .map_err(|e| {
            AudioCaptureError::StreamError(format!("Failed to register listener: {}", e))
        })?;

    // Build audio format parameters using PodSerializer
    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(AudioFormat::F32LE);

    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };

    let values: Vec<u8> = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .map_err(|e| AudioCaptureError::StreamError(format!("Failed to serialize params: {:?}", e)))?
        .0
        .into_inner();

    let mut params = [Pod::from_bytes(&values)
        .ok_or_else(|| AudioCaptureError::StreamError("Failed to create pod from bytes".into()))?];

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|e| AudioCaptureError::StreamError(format!("Failed to connect stream: {}", e)))?;

    log::info!("PipeWire audio capture started: {}", config.sink_name);

    main_loop.run();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = AudioCaptureConfig::default();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.sink_name, "Coyote Audio Capture");
    }

    #[test]
    fn test_audio_buffer_creation() {
        let buffer = AudioBuffer {
            samples: vec![0.0, 0.5, -0.5, 1.0],
            channels: 2,
            sample_rate: 48000,
        };
        assert_eq!(buffer.samples.len(), 4);
        assert_eq!(buffer.channels, 2);
    }
}
