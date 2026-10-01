use opencv::core::{Point, Point2f};

const INVALID_SCORE: f64 = 1.0e18;

/// The spatial moments used to score and locate a contour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContourMoments {
    pub m00: f64,
    pub m10: f64,
    pub m01: f64,
}

impl ContourMoments {
    pub fn area(self) -> f64 {
        self.m00.abs()
    }

    pub fn centroid(self) -> Option<Point2f> {
        if self.m00 == 0.0 || !self.m00.is_finite() {
            return None;
        }
        let x = (self.m10 / self.m00) as f32;
        let y = (self.m01 / self.m00) as f32;
        (x.is_finite() && y.is_finite()).then_some(Point2f::new(x, y))
    }
}

/// A contour accepted by the configured area limits, with its score and center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContourCandidate {
    pub index: usize,
    pub moments: ContourMoments,
    pub center: Point2f,
    pub score: f64,
}

/// Constant-velocity Kalman filter for tracking a point in screen coordinates.
///
/// The state is `[x, y, vx, vy]`, matching the filter in `detect.hpp`.
#[derive(Clone, Debug)]
pub struct KalmanPredictor {
    state: [f32; 4],
    covariance: [[f32; 4]; 4],
    initialized: bool,
}

impl Default for KalmanPredictor {
    fn default() -> Self {
        Self {
            state: [0.0; 4],
            covariance: [[0.0; 4]; 4],
            initialized: false,
        }
    }
}

impl KalmanPredictor {
    pub fn reset(&mut self) {
        self.initialized = false;
        self.state = [0.0; 4];
        self.covariance = [[0.0; 4]; 4];
    }

    /// Update the filter with a measured point and return the corrected estimate.
    /// The first measurement initializes the state. Non-finite measurements are
    /// ignored, and invalid/non-positive `dt` values use a one-second step.
    pub fn predict(&mut self, measured_x: f32, measured_y: f32, dt: f32) -> Point2f {
        if !measured_x.is_finite() || !measured_y.is_finite() {
            return if self.initialized {
                Point2f::new(self.state[0], self.state[1])
            } else {
                Point2f::new(0.0, 0.0)
            };
        }

        if !self.initialized {
            self.state = [measured_x, measured_y, 0.0, 0.0];
            self.covariance = [[0.0; 4]; 4];
            for i in 0..4 {
                self.covariance[i][i] = 1000.0;
            }
            self.initialized = true;
            return Point2f::new(measured_x, measured_y);
        }

        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 1.0 };
        let mut transition = [[0.0f32; 4]; 4];
        transition[0][0] = 1.0;
        transition[0][2] = dt;
        transition[1][1] = 1.0;
        transition[1][3] = dt;
        transition[2][2] = 1.0;
        transition[3][3] = 1.0;

        let mut predicted_state = [0.0f32; 4];
        for row in 0..4 {
            for col in 0..4 {
                predicted_state[row] += transition[row][col] * self.state[col];
            }
        }

        let mut predicted_covariance = [[0.0f32; 4]; 4];
        for row in 0..4 {
            for col in 0..4 {
                for a in 0..4 {
                    for b in 0..4 {
                        predicted_covariance[row][col] +=
                            transition[row][a] * self.covariance[a][b] * transition[col][b];
                    }
                }
            }
        }

        // Process noise covariance from the native constant-velocity filter.
        let mut process_noise = [[0.0f32; 4]; 4];
        process_noise[0][0] = 0.1;
        process_noise[0][2] = 0.05 * dt;
        process_noise[1][1] = 0.1;
        process_noise[1][3] = 0.05 * dt;
        process_noise[2][0] = 0.05 * dt;
        process_noise[2][2] = 0.1 * dt * dt;
        process_noise[3][1] = 0.05 * dt;
        process_noise[3][3] = 0.1 * dt * dt;
        for row in 0..4 {
            for col in 0..4 {
                predicted_covariance[row][col] += process_noise[row][col];
            }
        }

        // S = H P Hᵀ + R; H selects position and R is diag(2, 2).
        let s00 = predicted_covariance[0][0] + 2.0;
        let s01 = predicted_covariance[0][1];
        let s10 = predicted_covariance[1][0];
        let s11 = predicted_covariance[1][1] + 2.0;
        let determinant = s00 * s11 - s01 * s10;
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            self.state = predicted_state;
            self.covariance = predicted_covariance;
            return Point2f::new(self.state[0], self.state[1]);
        }

        let inv_s = [
            [s11 / determinant, -s01 / determinant],
            [-s10 / determinant, s00 / determinant],
        ];
        let mut gain = [[0.0f32; 2]; 4];
        for row in 0..4 {
            gain[row][0] = predicted_covariance[row][0] * inv_s[0][0]
                + predicted_covariance[row][1] * inv_s[1][0];
            gain[row][1] = predicted_covariance[row][0] * inv_s[0][1]
                + predicted_covariance[row][1] * inv_s[1][1];
        }

        let residual_x = measured_x - predicted_state[0];
        let residual_y = measured_y - predicted_state[1];
        for row in 0..4 {
            predicted_state[row] += gain[row][0] * residual_x + gain[row][1] * residual_y;
        }

        // P = (I - K H) P; H selects the first two state elements.
        let mut corrected_covariance = predicted_covariance;
        for row in 0..4 {
            for col in 0..4 {
                corrected_covariance[row][col] -= gain[row][0] * predicted_covariance[0][col]
                    + gain[row][1] * predicted_covariance[1][col];
            }
        }

        self.state = predicted_state;
        self.covariance = corrected_covariance;
        Point2f::new(self.state[0], self.state[1])
    }

    pub fn velocity(&self) -> Point2f {
        if self.initialized {
            Point2f::new(self.state[2], self.state[3])
        } else {
            Point2f::new(0.0, 0.0)
        }
    }
}

/// Compute polygon moments and the native contour score.
///
/// Contours must be ordered boundary points, as produced by OpenCV's
/// `find_contours`. Invalid, degenerate, or out-of-range contours receive
/// `1e18`, the same sentinel as `detect.hpp`.
pub fn score_contour(
    contour: &[Point],
    ref_x: f64,
    ref_y: f64,
    min_area: i32,
    max_area: i32,
    out_moments: Option<&mut ContourMoments>,
) -> f64 {
    let Some(moments) = contour_moments(contour) else {
        return INVALID_SCORE;
    };
    let area = moments.area();
    if !area.is_finite()
        || area < f64::from(min_area)
        || area > f64::from(max_area)
        || area == 0.0
        || !ref_x.is_finite()
        || !ref_y.is_finite()
    {
        return INVALID_SCORE;
    }

    let Some(center) = moments.centroid() else {
        return INVALID_SCORE;
    };
    if let Some(out) = out_moments {
        *out = moments;
    }

    let dx = f64::from(center.x) - ref_x;
    let dy = f64::from(center.y) - ref_y;
    dx * dx + dy * dy + 1.0 / (area + 1.0)
}

/// Return the OpenCV-style spatial moments for an ordered polygon contour.
pub fn contour_moments(contour: &[Point]) -> Option<ContourMoments> {
    if contour.len() < 3 {
        return None;
    }

    let mut twice_area = 0.0f64;
    let mut m10_times_six = 0.0f64;
    let mut m01_times_six = 0.0f64;
    for index in 0..contour.len() {
        let current = contour[index];
        let next = contour[(index + 1) % contour.len()];
        let x0 = f64::from(current.x);
        let y0 = f64::from(current.y);
        let x1 = f64::from(next.x);
        let y1 = f64::from(next.y);
        let cross = x0 * y1 - x1 * y0;
        twice_area += cross;
        m10_times_six += (x0 + x1) * cross;
        m01_times_six += (y0 + y1) * cross;
    }

    if twice_area == 0.0 || !twice_area.is_finite() {
        return None;
    }
    let moments = ContourMoments {
        m00: twice_area / 2.0,
        m10: m10_times_six / 6.0,
        m01: m01_times_six / 6.0,
    };
    moments.centroid().map(|_| moments)
}

/// Select the lowest-scoring valid contour, with distance dominant and area as
/// a small tie-breaker. Returns its original position in `contours`.
pub fn best_contour(
    contours: &[Vec<Point>],
    ref_x: f64,
    ref_y: f64,
    min_area: i32,
    max_area: i32,
) -> Option<ContourCandidate> {
    contours
        .iter()
        .enumerate()
        .filter_map(|(index, contour)| {
            let mut moments = ContourMoments::default();
            let score = score_contour(
                contour,
                ref_x,
                ref_y,
                min_area,
                max_area,
                Some(&mut moments),
            );
            if score >= INVALID_SCORE {
                return None;
            }
            Some(ContourCandidate {
                index,
                center: moments.centroid()?,
                moments,
                score,
            })
        })
        .min_by(|left, right| left.score.total_cmp(&right.score))
}
