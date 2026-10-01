use crate::config::{AimbotConfig, ColorSpace, parse_hex_color};
use crate::detect::{ContourCandidate, KalmanPredictor, best_contour};
use opencv::core::{self, Mat, Point, Point2f, Scalar, Vector};
use opencv::imgproc;
use opencv::prelude::*;
use std::fmt;

/// Supported packed 8-bit frame formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Bgr8,
    Rgb8,
    Bgra8,
    Rgba8,
}

impl PixelFormat {
    fn channels(self) -> usize {
        match self {
            Self::Bgr8 | Self::Rgb8 => 3,
            Self::Bgra8 | Self::Rgba8 => 4,
        }
    }
}

/// Borrowed CPU image data. `stride` is the byte distance between row starts.
pub struct CpuFrame<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
}

/// Target and suggested relative movement, in the frame's pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimResult {
    pub target: Point2f,
    pub predicted: Point2f,
    pub velocity: Point2f,
    pub movement: Point2f,
    pub distance: f32,
    pub strength: f32,
    pub score: f64,
}

#[derive(Debug)]
pub enum FrameError {
    Invalid(&'static str),
    OpenCv(opencv::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::OpenCv(error) => write!(f, "OpenCV: {error}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<opencv::Error> for FrameError {
    fn from(value: opencv::Error) -> Self {
        Self::OpenCv(value)
    }
}

#[derive(Clone, Copy)]
struct ColorThreshold {
    rgb: [u8; 3],
    bgr: [u8; 3],
    hsv: [u8; 3],
}

/// Stateful frame-to-target detector. The host supplies frames; this type does
/// not capture the desktop, own a thread, or send mouse/key input.
pub struct FrameDetector {
    config: AimbotConfig,
    colors: Vec<ColorThreshold>,
    kalman: KalmanPredictor,
    last_target: Option<Point2f>,
    lost_frames: u32,
}

impl FrameDetector {
    pub fn new(config: AimbotConfig) -> Result<Self, FrameError> {
        config
            .validate()
            .map_err(|_| FrameError::Invalid("invalid aimbot detector configuration"))?;
        let colors = config
            .colors
            .iter()
            .map(|color| {
                let rgb = parse_hex_color(color)
                    .ok_or(FrameError::Invalid("invalid configured color"))?;
                let bgr = [rgb[2], rgb[1], rgb[0]];
                Ok(ColorThreshold {
                    rgb,
                    bgr,
                    hsv: bgr_to_hsv(bgr),
                })
            })
            .collect::<Result<Vec<_>, FrameError>>()?;
        Ok(Self {
            config,
            colors,
            kalman: KalmanPredictor::default(),
            last_target: None,
            lost_frames: 0,
        })
    }

    pub fn reset(&mut self) {
        self.kalman.reset();
        self.last_target = None;
        self.lost_frames = 0;
    }

    /// Build the threshold mask and return the best detected target.
    /// `reference` is the frame-space aim point (normally the window center).
    pub fn process(
        &mut self,
        frame: CpuFrame<'_>,
        reference: Point2f,
        dt_seconds: f32,
    ) -> Result<Option<AimResult>, FrameError> {
        validate_frame(&frame)?;
        if !reference.x.is_finite() || !reference.y.is_finite() {
            return Err(FrameError::Invalid("reference point must be finite"));
        }
        if !self.config.enabled {
            self.reset();
            return Ok(None);
        }

        let mask = build_mask_validated(
            &frame,
            self.config.color_space,
            self.config.tolerance,
            self.config.tolerance_h,
            self.config.tolerance_s,
            self.config.tolerance_v,
            &self.colors,
        );
        let mut mask_mat = Mat::new_rows_cols_with_default(
            frame.height as i32,
            frame.width as i32,
            core::CV_8UC1,
            Scalar::all(0.0),
        )?;
        mask_mat.data_bytes_mut()?.copy_from_slice(&mask);

        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &mask_mat,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;
        let contour_points: Vec<Vec<Point>> = (0..contours.len())
            .map(|index| contours.get(index).map(|contour| contour.to_vec()))
            .collect::<opencv::Result<_>>()?;

        let reference_x = self
            .last_target
            .map_or(f64::from(reference.x), |point| f64::from(point.x));
        let reference_y = self
            .last_target
            .map_or(f64::from(reference.y), |point| f64::from(point.y));
        let candidate = best_contour(
            &contour_points,
            reference_x,
            reference_y,
            self.config.min_area,
            self.config.max_area,
        );
        let Some(candidate) = candidate else {
            self.note_lost_target();
            return Ok(None);
        };

        self.lost_frames = 0;
        self.last_target = Some(candidate.center);
        Ok(Some(self.make_result(candidate, reference, dt_seconds)))
    }

    fn note_lost_target(&mut self) {
        self.lost_frames = self.lost_frames.saturating_add(1);
        if self.lost_frames >= self.config.lost_frames_thresh {
            self.reset();
        }
    }

    fn make_result(
        &mut self,
        candidate: ContourCandidate,
        reference: Point2f,
        dt_seconds: f32,
    ) -> AimResult {
        let predicted = self
            .kalman
            .predict(candidate.center.x, candidate.center.y, dt_seconds);
        let velocity = self.kalman.velocity();
        let predicted = Point2f::new(
            predicted.x + self.config.offset_x,
            predicted.y + self.config.offset_y,
        );
        let dx = predicted.x - reference.x;
        let dy = predicted.y - reference.y;
        let distance = (dx * dx + dy * dy).sqrt();
        let strength = if distance < 0.5 {
            0.0
        } else {
            let t = (distance / self.config.fov).min(1.0);
            self.config.min_strength + t * (self.config.max_strength - self.config.min_strength)
        };
        let movement = Point2f::new((dx * strength).round(), (dy * strength).round());
        AimResult {
            target: candidate.center,
            predicted,
            velocity,
            movement,
            distance,
            strength,
            score: candidate.score,
        }
    }
}

/// Build a binary color mask from packed BGR/RGB/RGBA/BGRA pixels.
pub fn build_mask(frame: CpuFrame<'_>, config: &AimbotConfig) -> Result<Vec<u8>, FrameError> {
    validate_frame(&frame)?;
    config
        .validate()
        .map_err(|_| FrameError::Invalid("invalid aimbot detector configuration"))?;
    let colors = config
        .colors
        .iter()
        .map(|color| {
            let rgb =
                parse_hex_color(color).ok_or(FrameError::Invalid("invalid configured color"))?;
            let bgr = [rgb[2], rgb[1], rgb[0]];
            Ok(ColorThreshold {
                rgb,
                bgr,
                hsv: bgr_to_hsv(bgr),
            })
        })
        .collect::<Result<Vec<_>, FrameError>>()?;
    Ok(build_mask_validated(
        &frame,
        config.color_space,
        config.tolerance,
        config.tolerance_h,
        config.tolerance_s,
        config.tolerance_v,
        &colors,
    ))
}

fn validate_frame(frame: &CpuFrame<'_>) -> Result<(), FrameError> {
    if frame.width == 0 || frame.height == 0 {
        return Err(FrameError::Invalid("frame dimensions must be nonzero"));
    }
    if frame.width > i32::MAX as u32 || frame.height > i32::MAX as u32 {
        return Err(FrameError::Invalid("frame dimensions exceed OpenCV limits"));
    }
    let row_bytes = (frame.width as usize)
        .checked_mul(frame.format.channels())
        .ok_or(FrameError::Invalid("frame row size overflow"))?;
    if frame.stride < row_bytes {
        return Err(FrameError::Invalid(
            "frame stride is shorter than a pixel row",
        ));
    }
    let required = frame
        .stride
        .checked_mul(frame.height as usize - 1)
        .and_then(|last_row| last_row.checked_add(row_bytes))
        .ok_or(FrameError::Invalid("frame buffer size overflow"))?;
    if frame.pixels.len() < required {
        return Err(FrameError::Invalid("frame pixel buffer is truncated"));
    }
    Ok(())
}

fn build_mask_validated(
    frame: &CpuFrame<'_>,
    color_space: ColorSpace,
    tolerance: f32,
    tolerance_h: f32,
    tolerance_s: f32,
    tolerance_v: f32,
    colors: &[ColorThreshold],
) -> Vec<u8> {
    let mut mask = vec![0; frame.width as usize * frame.height as usize];
    let channels = frame.format.channels();
    for y in 0..frame.height as usize {
        let row_start = y * frame.stride;
        for x in 0..frame.width as usize {
            let start = row_start + x * channels;
            let pixel = &frame.pixels[start..start + channels];
            let bgr = match frame.format {
                PixelFormat::Bgr8 | PixelFormat::Bgra8 => [pixel[0], pixel[1], pixel[2]],
                PixelFormat::Rgb8 | PixelFormat::Rgba8 => [pixel[2], pixel[1], pixel[0]],
            };
            let included = colors.iter().any(|color| match color_space {
                ColorSpace::Bgr => near_channels(&bgr, &color.bgr, tolerance),
                ColorSpace::Rgb => {
                    let rgb = [bgr[2], bgr[1], bgr[0]];
                    near_channels(&rgb, &color.rgb, tolerance)
                }
                ColorSpace::Hsv => {
                    let hsv = bgr_to_hsv(bgr);
                    let hue_delta = hsv[0]
                        .abs_diff(color.hsv[0])
                        .min(180u8.saturating_sub(hsv[0].abs_diff(color.hsv[0])));
                    f32::from(hue_delta) <= tolerance_h
                        && f32::from(hsv[1].abs_diff(color.hsv[1])) <= tolerance_s
                        && f32::from(hsv[2].abs_diff(color.hsv[2])) <= tolerance_v
                }
            });
            mask[y * frame.width as usize + x] = if included { 255 } else { 0 };
        }
    }
    mask
}

fn near_channels(pixel: &[u8; 3], target: &[u8; 3], tolerance: f32) -> bool {
    (0..3).all(|channel| f32::from(pixel[channel].abs_diff(target[channel])) <= tolerance)
}

fn bgr_to_hsv([b, g, r]: [u8; 3]) -> [u8; 3] {
    let b = f32::from(b) / 255.0;
    let g = f32::from(g) / 255.0;
    let r = f32::from(r) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue_degrees = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let hue = (hue_degrees / 2.0).round().rem_euclid(180.0) as u8;
    let saturation = if max == 0.0 { 0.0 } else { delta / max * 255.0 };
    [hue, saturation.round() as u8, (max * 255.0).round() as u8]
}
