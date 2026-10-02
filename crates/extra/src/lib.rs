//! Color-based frame detection for the runtime's optional detection overlay.

mod config;
mod detector;

pub use config::{DetectionConfig, DetectionConfigError, DetectionType};
pub use detector::{BoundingBox, Detection, Detector, Point};
pub use opencv;
