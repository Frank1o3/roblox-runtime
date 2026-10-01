use serde::{Deserialize, Serialize};
use std::fmt;

/// Color channels used while building the detector mask.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorSpace {
    #[default]
    Bgr,
    Rgb,
    Hsv,
}

/// Runtime detector and tracking options. Capture-source settings deliberately
/// live outside this type because frames are supplied by the host runtime.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AimbotConfig {
    pub enabled: bool,
    pub debug: bool,
    pub colors: Vec<String>,
    pub color_space: ColorSpace,
    pub tolerance: f32,
    pub tolerance_h: f32,
    pub tolerance_s: f32,
    pub tolerance_v: f32,
    pub min_area: i32,
    pub max_area: i32,
    pub fov: f32,
    pub smoothness: f32,
    pub min_strength: f32,
    pub max_strength: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub lost_frames_thresh: u32,
}

impl Default for AimbotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            debug: false,
            colors: vec!["#ffffb2".into(), "#ffffb4".into(), "#ffffb6".into()],
            color_space: ColorSpace::Bgr,
            tolerance: 15.0,
            tolerance_h: 10.0,
            tolerance_s: 60.0,
            tolerance_v: 60.0,
            min_area: 1,
            max_area: 9999,
            fov: 350.0,
            smoothness: 0.1,
            min_strength: 0.033,
            max_strength: 0.8,
            offset_x: 0.0,
            offset_y: 0.0,
            lost_frames_thresh: 4,
        }
    }
}

impl AimbotConfig {
    /// Validate values before constructing a detector. A bad configuration is
    /// reported instead of silently turning into an unrestricted threshold.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.colors.is_empty() {
            return Err(ConfigError("colors must contain at least one RGB hex color"));
        }
        if self.colors.iter().any(|color| parse_hex_color(color).is_none()) {
            return Err(ConfigError("colors must use #RRGGBB or RRGGBB format"));
        }
        if [
            self.tolerance,
            self.tolerance_h,
            self.tolerance_s,
            self.tolerance_v,
            self.fov,
            self.smoothness,
            self.min_strength,
            self.max_strength,
            self.offset_x,
            self.offset_y,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return Err(ConfigError("all numeric detector settings must be finite"));
        }
        if self.min_area < 1 || self.max_area < self.min_area {
            return Err(ConfigError("area limits must satisfy 1 <= min_area <= max_area"));
        }
        if self.fov <= 0.0 {
            return Err(ConfigError("fov must be greater than zero"));
        }
        if !(0.0..=255.0).contains(&self.tolerance)
            || !(0.0..=90.0).contains(&self.tolerance_h)
            || !(0.0..=255.0).contains(&self.tolerance_s)
            || !(0.0..=255.0).contains(&self.tolerance_v)
            || !(0.0..=1.0).contains(&self.smoothness)
            || self.min_strength < 0.0
            || self.max_strength < self.min_strength
            || self.max_strength > 1.0
            || self.lost_frames_thresh == 0
        {
            return Err(ConfigError("aim and tracking limits are invalid"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfigError(&'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Parse the app's `#RRGGBB` color spelling into `(red, green, blue)` bytes.
pub fn parse_hex_color(color: &str) -> Option<[u8; 3]> {
    let digits = color.strip_prefix('#').unwrap_or(color);
    if digits.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some([
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ])
}
