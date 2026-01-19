mod analysis;
mod mapper;
mod pipewire;

pub use analysis::{
    AnalysisMode, AnalysisResult, AudioAnalyzer, BeatDetectionConfig, ChannelResult, FrequencyBands,
    SPECTRUM_BARS,
};
pub use mapper::{AudioMapper, CoyoteCommand, MapperConfig, MappingCurve};
pub use pipewire::{AudioBuffer, AudioCapture, AudioCaptureConfig, AudioCaptureError};
