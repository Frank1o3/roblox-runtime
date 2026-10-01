pub mod config;
pub mod detect;
pub mod frame;

#[cfg(test)]
mod tests {
    use crate::config::{AimbotConfig, ColorSpace};
    use crate::detect::{KalmanPredictor, best_contour, score_contour};
    use crate::frame::{CpuFrame, FrameDetector, PixelFormat, build_mask};
    use opencv::core::{Point, Point2f};

    #[test]
    fn kalman_predictor_initializes_and_tracks_motion() {
        let mut k = KalmanPredictor::default();
        let first = k.predict(10.0, 20.0, 1.0);
        let second = k.predict(15.0, 22.0, 1.0);

        assert!((first.x - 10.0).abs() < 1e-5);
        assert!((first.y - 20.0).abs() < 1e-5);
        assert!(second.x > first.x);
        assert!(second.y > first.y);
    }

    #[test]
    fn contour_score_prefers_near_target_and_valid_area() {
        let square = vec![
            Point::new(0, 0),
            Point::new(20, 0),
            Point::new(20, 20),
            Point::new(0, 20),
        ];
        let score = score_contour(&square, 12.0, 12.0, 10, 1000, None);
        assert!(score.is_finite());
        assert!(score < 1000.0);
    }

    #[test]
    fn best_contour_uses_distance_and_rejects_invalid_area() {
        let near = vec![
            Point::new(8, 8),
            Point::new(12, 8),
            Point::new(12, 12),
            Point::new(8, 12),
        ];
        let far = vec![
            Point::new(40, 40),
            Point::new(60, 40),
            Point::new(60, 60),
            Point::new(40, 60),
        ];
        let too_small = vec![
            Point::new(0, 0),
            Point::new(1, 0),
            Point::new(1, 1),
            Point::new(0, 1),
        ];

        let candidate = best_contour(&[far, too_small, near], 10.0, 10.0, 4, 500).unwrap();
        assert_eq!(candidate.index, 2);
        assert_eq!(candidate.center.x, 10.0);
        assert_eq!(candidate.center.y, 10.0);
    }

    #[test]
    fn config_defaults_and_json_round_trip() {
        let config: AimbotConfig = serde_json::from_str("{}").unwrap();
        assert!(!config.enabled);
        assert!(!config.colors.is_empty());
        assert!(config.validate().is_ok());

        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: AimbotConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.colors, config.colors);
        assert_eq!(decoded.min_area, config.min_area);
    }

    #[test]
    fn mask_respects_stride_and_pixel_format() {
        let mut config = AimbotConfig::default();
        config.colors = vec!["#ff0000".into()];
        config.color_space = ColorSpace::Bgr;
        config.tolerance = 0.0;
        // Two BGR pixels followed by two bytes of row padding.
        let pixels = [0, 0, 255, 0, 0, 0, 9, 9];
        let mask = build_mask(
            CpuFrame {
                pixels: &pixels,
                width: 2,
                height: 1,
                stride: 8,
                format: PixelFormat::Bgr8,
            },
            &config,
        )
        .unwrap();
        assert_eq!(mask, [255, 0]);
    }

    #[test]
    fn detector_finds_target_and_resets_after_lost_threshold() {
        let mut config = AimbotConfig::default();
        config.enabled = true;
        config.colors = vec!["#ff0000".into()];
        config.tolerance = 0.0;
        config.min_area = 1;
        config.lost_frames_thresh = 2;
        let mut detector = FrameDetector::new(config.clone()).unwrap();

        let mut pixels = vec![0; 16 * 16 * 3];
        for y in 6..10 {
            for x in 6..10 {
                let offset = (y * 16 + x) * 3;
                pixels[offset..offset + 3].copy_from_slice(&[0, 0, 255]);
            }
        }
        let result = detector
            .process(
                CpuFrame {
                    pixels: &pixels,
                    width: 16,
                    height: 16,
                    stride: 16 * 3,
                    format: PixelFormat::Bgr8,
                },
                Point2f::new(8.0, 8.0),
                1.0 / 60.0,
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.target.x, 7.5);
        assert_eq!(result.target.y, 7.5);

        let blank = vec![0; 16 * 16 * 3];
        for _ in 0..config.lost_frames_thresh {
            assert!(
                detector
                    .process(
                        CpuFrame {
                            pixels: &blank,
                            width: 16,
                            height: 16,
                            stride: 16 * 3,
                            format: PixelFormat::Bgr8,
                        },
                        Point2f::new(8.0, 8.0),
                        1.0 / 60.0,
                    )
                    .unwrap()
                    .is_none()
            );
        }
    }
}
