pub mod detect;

#[cfg(test)]
mod tests {
    use crate::detect::{KalmanPredictor, best_contour, score_contour};
    use opencv::core::Point;

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
}
