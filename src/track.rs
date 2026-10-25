use glam::Vec2;

/// A path through Catmull-Rom control points (closed loop or open road).
pub struct Track {
    pub points_2d: Vec<Vec2>,
    pub closed: bool,
}

impl Track {
    pub fn from_control_points(control_points: &[Vec2], closed: bool) -> Self {
        let mut track_points_2d = Vec::new();

        let num_points = control_points.len();
        let segments_per_section = 10;

        for i in 0..num_points {
            let p0 = if closed {
                control_points[(i + num_points - 1) % num_points]
            } else if i == 0 {
                control_points[0]
            } else {
                control_points[i - 1]
            };

            let p1 = control_points[i];

            let p2 = if i < num_points - 1 {
                control_points[i + 1]
            } else if closed {
                control_points[0]
            } else {
                control_points[num_points - 1]
            };

            let p3 = if i < num_points - 2 {
                control_points[i + 2]
            } else if closed {
                control_points[(i + 2) % num_points]
            } else {
                control_points[num_points - 1]
            };

            if !closed && i == num_points - 1 {
                break;
            }

            for j in 0..segments_per_section {
                let t = j as f32 / segments_per_section as f32;
                let point = catmull_rom_2d(p0, p1, p2, p3, t);
                track_points_2d.push(point);
            }
        }

        if !closed {
            track_points_2d.push(control_points[num_points - 1]);
        }
        Track {
            points_2d: track_points_2d,
            closed,
        }
    }
}

fn catmull_rom_2d(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    let v0 = (p2 - p0) * 0.5;
    let v1 = (p3 - p1) * 0.5;
    (2.0 * p1 - 2.0 * p2 + v0 + v1) * t3
        + (-3.0 * p1 + 3.0 * p2 - 2.0 * v0 - v1) * t2
        + v0 * t
        + p1
}
