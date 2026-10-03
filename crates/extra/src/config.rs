use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectionType {
    Rgb,
    #[default]
    Bgr,
    Hsv,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DetectionConfig {
    /// Width and height of the centered square region analyzed, in pixels.
    pub fov: i32,
    /// Per-channel tolerance for RGB and BGR matching (0..=255).
    pub tolerance: f64,
    /// HSV tolerances; OpenCV hue is 0..=179 and saturation/value are 0..=255.
    pub tolerance_h: f64,
    pub tolerance_s: f64,
    pub tolerance_v: f64,
    pub min_area: f64,
    pub max_area: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// EMA decay for movement smoothing; 0 is responsive and 1 retains motion.
    pub smoothness: f64,
    /// Movement gain at the centre of the FOV.
    pub min_strength: f64,
    /// Movement gain at the edge of the FOV.
    pub max_strength: f64,
    /// Fraction of the last frame's movement added to the reported point.
    pub lead: f64,
    /// Whether aim assist starts enabled (F1 continues to toggle it at runtime).
    pub enabled: bool,
    /// Require the right mouse button to be held while aim assist is active.
    pub aimbot_requires_trigger: bool,
    /// Whether steady aim starts enabled (F4 continues to toggle it at runtime).
    pub steady_aim: bool,
    /// Radius from screen centre in pixels at which steady aim holds its key.
    pub steady_dist: f64,
    /// Keyboard key held while a target is within steady_dist.
    pub steady_key: String,
    /// Whether triggerbot starts enabled (F3 continues to toggle it at runtime).
    pub triggerbot: bool,
    /// Maximum distance from frame centre at which triggerbot may click.
    pub trigger_dist: f64,
    /// Minimum interval between trigger clicks, in milliseconds.
    pub trigger_delay: u64,
    /// Hex RGB colors such as `#ffffb2`. Multiple colors are ORed together.
    pub colors: Vec<String>,
    #[serde(alias = "color_space")]
    pub detection_type: DetectionType,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            fov: 350,
            tolerance: 15.0,
            tolerance_h: 10.0,
            tolerance_s: 60.0,
            tolerance_v: 60.0,
            min_area: 1.0,
            max_area: 9999.0,
            offset_x: 0.0,
            offset_y: 0.0,
            smoothness: 0.1,
            min_strength: 0.033,
            max_strength: 0.8,
            lead: 0.0,
            enabled: false,
            aimbot_requires_trigger: false,
            steady_aim: false,
            steady_dist: 24.0,
            steady_key: "left_shift".into(),
            triggerbot: false,
            trigger_dist: 10.0,
            trigger_delay: 250,
            colors: vec!["#ffffb2".into(), "#ffffb4".into(), "#ffffb6".into()],
            detection_type: DetectionType::Bgr,
        }
    }
}

impl DetectionConfig {
    /// Read the config, creating a default file on first launch.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, DetectionConfigError> {
        let path = path.as_ref();
        match std::fs::read(path) {
            Ok(bytes) => {
                let config: Self = serde_json::from_slice(&bytes)?;
                config.validate()?;
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.save(path)?;
                Ok(config)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Save through a sibling temporary file so a partial write cannot corrupt the config.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), DetectionConfigError> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), DetectionConfigError> {
        if self.fov <= 0 {
            return Err(DetectionConfigError::Invalid(
                "fov must be greater than zero",
            ));
        }
        if !self.min_area.is_finite() || self.min_area < 0.0 {
            return Err(DetectionConfigError::Invalid(
                "min_area must be finite and non-negative",
            ));
        }
        if !self.max_area.is_finite() || self.max_area < self.min_area {
            return Err(DetectionConfigError::Invalid(
                "max_area must be finite and at least min_area",
            ));
        }
        for (name, value) in [
            ("tolerance", self.tolerance),
            ("tolerance_h", self.tolerance_h),
            ("tolerance_s", self.tolerance_s),
            ("tolerance_v", self.tolerance_v),
            ("offset_x", self.offset_x),
            ("offset_y", self.offset_y),
            ("lead", self.lead),
            ("smoothness", self.smoothness),
            ("min_strength", self.min_strength),
            ("max_strength", self.max_strength),
            ("trigger_dist", self.trigger_dist),
            ("steady_dist", self.steady_dist),
        ] {
            if !value.is_finite() {
                return Err(DetectionConfigError::InvalidField(name));
            }
        }
        if self.tolerance < 0.0
            || self.tolerance > 255.0
            || self.tolerance_h < 0.0
            || self.tolerance_h > 90.0
            || self.tolerance_s < 0.0
            || self.tolerance_s > 255.0
            || self.tolerance_v < 0.0
            || self.tolerance_v > 255.0
            || self.lead < 0.0
            || !(0.0..=1.0).contains(&self.smoothness)
            || self.min_strength < 0.0
            || self.max_strength < 0.0
            || self.max_strength < self.min_strength
            || self.trigger_dist < 0.0
            || self.steady_dist < 0.0
        {
            return Err(DetectionConfigError::Invalid(
                "tolerances and movement settings are out of range",
            ));
        }
        if !matches!(
            self.steady_key
                .trim()
                .to_ascii_lowercase()
                .replace('-', "_")
                .replace(' ', "_")
                .as_str(),
            "left_shift"
                | "lshift"
                | "shift"
                | "right_shift"
                | "rshift"
                | "left_ctrl"
                | "lctrl"
                | "ctrl"
                | "control"
                | "right_ctrl"
                | "rctrl"
                | "left_alt"
                | "lalt"
                | "alt"
                | "right_alt"
                | "ralt"
                | "space"
                | "spacebar"
        ) {
            return Err(DetectionConfigError::Invalid(
                "steady_key must be a supported modifier key or space",
            ));
        }
        if self.colors.is_empty() {
            return Err(DetectionConfigError::Invalid(
                "colors must contain at least one hex color",
            ));
        }
        for color in &self.colors {
            parse_rgb(color)?;
        }
        Ok(())
    }
}

pub(crate) fn parse_rgb(color: &str) -> Result<[u8; 3], DetectionConfigError> {
    let value = color.strip_prefix('#').unwrap_or(color);
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DetectionConfigError::InvalidColor(color.to_owned()));
    }
    let rgb = u32::from_str_radix(value, 16)
        .map_err(|_| DetectionConfigError::InvalidColor(color.to_owned()))?;
    Ok([
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    ])
}

#[derive(Debug)]
pub enum DetectionConfigError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(&'static str),
    InvalidField(&'static str),
    InvalidColor(String),
}

impl std::fmt::Display for DetectionConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "detection config I/O: {error}"),
            Self::Json(error) => write!(formatter, "detection config JSON: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid detection config: {message}"),
            Self::InvalidField(field) => {
                write!(formatter, "detection config {field} must be finite")
            }
            Self::InvalidColor(color) => {
                write!(formatter, "invalid hex color {color:?}; expected #RRGGBB")
            }
        }
    }
}

impl std::error::Error for DetectionConfigError {}

impl From<std::io::Error> for DetectionConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for DetectionConfigError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
