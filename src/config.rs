use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use thiserror::Error;

use crate::audio::MappingCurve;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to determine config directory")]
    NoConfigDir,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_max_intensity")]
    pub max_intensity_a: u16,
    #[serde(default = "default_max_intensity")]
    pub max_intensity_b: u16,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,
    #[serde(default = "default_freq_band_min")]
    pub freq_band_min: f32,
    #[serde(default = "default_freq_band_max")]
    pub freq_band_max: f32,
    #[serde(default)]
    pub last_device_address: Option<String>,
    #[serde(default)]
    pub mapping_curve: MappingCurve,
    #[serde(default)]
    pub show_spectrum_analyzer: bool,
}

fn default_max_intensity() -> u16 {
    1024
}

fn default_sensitivity() -> f32 {
    0.5
}

fn default_freq_band_min() -> f32 {
    200.0
}

fn default_freq_band_max() -> f32 {
    800.0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_intensity_a: default_max_intensity(),
            max_intensity_b: default_max_intensity(),
            sensitivity: default_sensitivity(),
            freq_band_min: default_freq_band_min(),
            freq_band_max: default_freq_band_max(),
            last_device_address: None,
            mapping_curve: MappingCurve::default(),
            show_spectrum_analyzer: false,
        }
    }
}


impl Config {
    fn config_path() -> Result<PathBuf, ConfigError> {
        ProjectDirs::from("", "", "coyote-audio")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .ok_or(ConfigError::NoConfigDir)
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path()?;

        if !path.exists() {
            log::info!("Config file not found at {:?}, using defaults", path);
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&contents)?;

        log::info!("Loaded config from {:?}", path);
        Ok(config)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::config_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents)?;

        log::info!("Saved config to {:?}", path);
        Ok(())
    }

    pub fn set_max_intensity_a(&mut self, value: u16) {
        self.max_intensity_a = value.min(2047);
    }

    pub fn set_max_intensity_b(&mut self, value: u16) {
        self.max_intensity_b = value.min(2047);
    }

    pub fn set_sensitivity(&mut self, value: f32) {
        self.sensitivity = value.clamp(0.0, 1.0);
    }

    pub fn set_freq_band_min(&mut self, value: f32) {
        self.freq_band_min = value.clamp(20.0, self.freq_band_max - 10.0);
    }

    pub fn set_freq_band_max(&mut self, value: f32) {
        self.freq_band_max = value.clamp(self.freq_band_min + 10.0, 2000.0);
    }

    pub fn set_mapping_curve(&mut self, curve: MappingCurve) {
        self.mapping_curve = curve;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.max_intensity_a, 1024);
        assert_eq!(config.max_intensity_b, 1024);
        assert_eq!(config.sensitivity, 0.5);
        assert_eq!(config.freq_band_min, 200.0);
        assert_eq!(config.freq_band_max, 800.0);
        assert!(config.last_device_address.is_none());
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = Config {
            max_intensity_a: 500,
            max_intensity_b: 750,
            sensitivity: 0.8,
            freq_band_min: 100.0,
            freq_band_max: 600.0,
            last_device_address: Some("AA:BB:CC:DD:EE:FF".to_string()),
            mapping_curve: MappingCurve::Exponential,
            show_spectrum_analyzer: true,
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.max_intensity_a, 500);
        assert_eq!(parsed.max_intensity_b, 750);
        assert_eq!(parsed.sensitivity, 0.8);
        assert_eq!(parsed.freq_band_min, 100.0);
        assert_eq!(parsed.freq_band_max, 600.0);
        assert_eq!(
            parsed.last_device_address,
            Some("AA:BB:CC:DD:EE:FF".to_string())
        );
        assert_eq!(parsed.show_spectrum_analyzer, true);
    }

    #[test]
    fn test_set_max_intensity_clamped() {
        let mut config = Config::default();
        config.set_max_intensity_a(3000);
        assert_eq!(config.max_intensity_a, 2047);
    }

    #[test]
    fn test_set_sensitivity_clamped() {
        let mut config = Config::default();

        config.set_sensitivity(1.5);
        assert_eq!(config.sensitivity, 1.0);

        config.set_sensitivity(-0.5);
        assert_eq!(config.sensitivity, 0.0);
    }

    #[test]
    fn test_set_freq_band_clamped() {
        let mut config = Config::default();

        config.set_freq_band_min(10.0);
        assert_eq!(config.freq_band_min, 20.0);

        config.set_freq_band_max(3000.0);
        assert_eq!(config.freq_band_max, 2000.0);

        config.set_freq_band_min(1990.0);
        assert_eq!(config.freq_band_min, 1990.0);

        config.set_freq_band_max(1995.0);
        assert_eq!(config.freq_band_max, 2000.0);
    }
}
