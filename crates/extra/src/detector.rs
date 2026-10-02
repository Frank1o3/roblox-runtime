use crate::config::{DetectionConfig, DetectionType, parse_rgb};
use opencv::core::{self, Mat, Point as CvPoint, Scalar, Size, Vector};
use opencv::prelude::*;
use opencv::{geometry, imgproc};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BoundingBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub bounds: BoundingBox,
    pub center: Point,
    pub adjusted_center: Point,
    pub area: f64,
}

/// Stateful color detector. Keep one instance on the frame-processing thread
/// so `lead` and target continuity can use the previous frame's detection.
#[derive(Default)]
pub struct Detector {
    previous_center: Option<Point>,
    smoothed_movement: Option<Point>,
}

impl Detector {
    pub fn reset(&mut self) {
        self.previous_center = None;
        self.smoothed_movement = None;
    }

    /// Detect the best matching color blob in a CV_8UC3 BGR frame.
    ///
    /// `detection_type` selects the color space used for comparison; the frame
    /// input remains BGR, matching OpenCV's usual image convention.
    pub fn detect(
        &mut self,
        frame_bgr: &Mat,
        config: &DetectionConfig,
    ) -> opencv::Result<Option<Detection>> {
        config
            .validate()
            .map_err(|error| opencv::Error::new(core::StsBadArg, error.to_string()))?;
        if frame_bgr.empty() || frame_bgr.typ() != core::CV_8UC3 {
            return Err(opencv::Error::new(
                core::StsBadArg,
                "frame must be a non-empty CV_8UC3 BGR image",
            ));
        }

        let width = frame_bgr.cols();
        let height = frame_bgr.rows();
        let roi_width = config.fov.min(width);
        let roi_height = config.fov.min(height);
        let roi_x = (width - roi_width) / 2;
        let roi_y = (height - roi_height) / 2;
        let roi = frame_bgr.roi(core::Rect::new(roi_x, roi_y, roi_width, roi_height))?;

        let mut comparison = Mat::default();
        match config.detection_type {
            DetectionType::Bgr => roi.copy_to(&mut comparison)?,
            DetectionType::Rgb => {
                imgproc::cvt_color_def(&roi, &mut comparison, imgproc::COLOR_BGR2RGB)?
            }
            DetectionType::Hsv => {
                imgproc::cvt_color_def(&roi, &mut comparison, imgproc::COLOR_BGR2HSV)?
            }
        }

        let mut combined = Mat::default();
        for (color_index, color) in config.colors.iter().enumerate() {
            let rgb = parse_rgb(color)
                .map_err(|error| opencv::Error::new(core::StsBadArg, error.to_string()))?;
            let mut color_mask = Mat::default();
            match config.detection_type {
                DetectionType::Bgr | DetectionType::Rgb => {
                    let channels = match config.detection_type {
                        DetectionType::Bgr => [rgb[2], rgb[1], rgb[0]],
                        DetectionType::Rgb => rgb,
                        DetectionType::Hsv => unreachable!(),
                    };
                    let tolerance = config.tolerance;
                    core::in_range(
                        &comparison,
                        &Scalar::new(
                            (channels[0] as f64 - tolerance).max(0.0),
                            (channels[1] as f64 - tolerance).max(0.0),
                            (channels[2] as f64 - tolerance).max(0.0),
                            0.0,
                        ),
                        &Scalar::new(
                            (channels[0] as f64 + tolerance).min(255.0),
                            (channels[1] as f64 + tolerance).min(255.0),
                            (channels[2] as f64 + tolerance).min(255.0),
                            0.0,
                        ),
                        &mut color_mask,
                    )?;
                }
                DetectionType::Hsv => {
                    let pixel = Mat::new_rows_cols_with_default(
                        1,
                        1,
                        core::CV_8UC3,
                        Scalar::new(rgb[2] as f64, rgb[1] as f64, rgb[0] as f64, 0.0),
                    )?;
                    let mut hsv_pixel = Mat::default();
                    imgproc::cvt_color_def(&pixel, &mut hsv_pixel, imgproc::COLOR_BGR2HSV)?;
                    let hsv = *hsv_pixel.at_2d::<core::Vec3b>(0, 0)?;
                    let hue = hsv[0] as f64;
                    let saturation = hsv[1] as f64;
                    let value = hsv[2] as f64;
                    let low_h = hue - config.tolerance_h;
                    let high_h = hue + config.tolerance_h;
                    let low_s = (saturation - config.tolerance_s).max(0.0);
                    let high_s = (saturation + config.tolerance_s).min(255.0);
                    let low_v = (value - config.tolerance_v).max(0.0);
                    let high_v = (value + config.tolerance_v).min(255.0);

                    if low_h < 0.0 || high_h > 179.0 {
                        let (first_low, first_high, second_low, second_high) = if low_h < 0.0 {
                            (0.0, high_h, 180.0 + low_h, 179.0)
                        } else {
                            (low_h, 179.0, 0.0, high_h - 180.0)
                        };
                        let mut first = Mat::default();
                        let mut second = Mat::default();
                        core::in_range(
                            &comparison,
                            &Scalar::new(first_low, low_s, low_v, 0.0),
                            &Scalar::new(first_high, high_s, high_v, 0.0),
                            &mut first,
                        )?;
                        core::in_range(
                            &comparison,
                            &Scalar::new(second_low, low_s, low_v, 0.0),
                            &Scalar::new(second_high, high_s, high_v, 0.0),
                            &mut second,
                        )?;
                        core::bitwise_or(&first, &second, &mut color_mask, &core::no_array())?;
                    } else {
                        core::in_range(
                            &comparison,
                            &Scalar::new(low_h, low_s, low_v, 0.0),
                            &Scalar::new(high_h, high_s, high_v, 0.0),
                            &mut color_mask,
                        )?;
                    }
                }
            }

            if color_index == 0 {
                combined = color_mask;
            } else {
                let mut merged = Mat::default();
                core::bitwise_or(&combined, &color_mask, &mut merged, &core::no_array())?;
                combined = merged;
            }
        }

        let kernel = imgproc::get_structuring_element(
            imgproc::MORPH_ELLIPSE,
            Size::new(3, 3),
            CvPoint::new(-1, -1),
        )?;
        let mut cleaned = Mat::default();
        imgproc::morphology_ex(
            &combined,
            &mut cleaned,
            imgproc::MORPH_OPEN,
            &kernel,
            CvPoint::new(-1, -1),
            1,
            core::BORDER_CONSTANT,
            imgproc::morphology_default_border_value()?,
        )?;

        let mut contours = Vector::<Vector<CvPoint>>::new();
        imgproc::find_contours(
            &cleaned,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            CvPoint::new(0, 0),
        )?;

        let frame_center = Point {
            x: width as f32 / 2.0,
            y: height as f32 / 2.0,
        };
        let reference = self.previous_center.unwrap_or(frame_center);
        let mut best: Option<(f64, Detection)> = None;
        for contour_index in 0..contours.len() {
            let contour = contours.get(contour_index)?;
            let area = geometry::contour_area(&contour, false)?;
            if area < config.min_area || area > config.max_area || area <= 0.0 {
                continue;
            }
            let moments = geometry::moments(&contour, false)?;
            if moments.m00 == 0.0 {
                continue;
            }
            let center = Point {
                x: (moments.m10 / moments.m00) as f32 + roi_x as f32,
                y: (moments.m01 / moments.m00) as f32 + roi_y as f32,
            };
            let bounds = geometry::bounding_rect(&contour)?;
            let bounds = BoundingBox {
                x: bounds.x + roi_x,
                y: bounds.y + roi_y,
                width: bounds.width,
                height: bounds.height,
            };
            let dx = center.x - reference.x;
            let dy = center.y - reference.y;
            let score = (dx * dx + dy * dy) as f64 + 1.0 / (area + 1.0);
            let predicted = Point {
                x: center.x
                    + self
                        .previous_center
                        .map_or(0.0, |previous| (center.x - previous.x) * config.lead as f32)
                    + config.offset_x as f32,
                y: center.y
                    + self
                        .previous_center
                        .map_or(0.0, |previous| (center.y - previous.y) * config.lead as f32)
                    + config.offset_y as f32,
            };
            let raw_dx = predicted.x - frame_center.x;
            let raw_dy = predicted.y - frame_center.y;
            let distance = (raw_dx * raw_dx + raw_dy * raw_dy).sqrt();
            let strength = config.min_strength as f32
                + (distance / config.fov as f32).min(1.0)
                    * (config.max_strength - config.min_strength) as f32;
            let movement = Point {
                x: raw_dx * strength,
                y: raw_dy * strength,
            };
            let alpha = config.smoothness as f32;
            let movement = self.smoothed_movement.map_or(movement, |previous| Point {
                x: alpha * previous.x + (1.0 - alpha) * movement.x,
                y: alpha * previous.y + (1.0 - alpha) * movement.y,
            });
            let detection = Detection {
                bounds,
                center,
                adjusted_center: Point {
                    x: frame_center.x + movement.x,
                    y: frame_center.y + movement.y,
                },
                area,
            };
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score < *best_score)
            {
                best = Some((score, detection));
            }
        }

        let Some((_, detection)) = best else {
            self.previous_center = None;
            self.smoothed_movement = None;
            return Ok(None);
        };

        self.previous_center = Some(detection.center);
        self.smoothed_movement = Some(Point {
            x: detection.adjusted_center.x - frame_center.x,
            y: detection.adjusted_center.y - frame_center.y,
        });
        Ok(Some(detection))
    }
}
