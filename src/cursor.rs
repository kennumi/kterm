//! Kitty-style cursor trail: the four corners of the cursor chase their target at
//! different speeds. Corners on the leading edge (direction of travel) are fast,
//! trailing corners are slow, so a jump leaves a stretched, shrinking smear.

/// Seconds for the leading / trailing corners to (nearly) reach the target. Kitty's defaults.
const DECAY_FAST: f32 = 0.10;
const DECAY_SLOW: f32 = 0.40;
/// Jumps shorter than this many cells snap instantly, so typing doesn't smear.
const START_THRESHOLD_CELLS: f32 = 2.0;

pub type Quad = [[f32; 2]; 4]; // tl, tr, br, bl

pub struct Trail {
    pub corners: Quad,
    target: Quad,
    pub active: bool,
}

fn quad(x: f32, y: f32, w: f32, h: f32) -> Quad {
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

fn center(q: &Quad) -> [f32; 2] {
    [(q[0][0] + q[2][0]) * 0.5, (q[0][1] + q[2][1]) * 0.5]
}

impl Trail {
    pub fn new() -> Self {
        Self { corners: [[0.0; 2]; 4], target: [[0.0; 2]; 4], active: false }
    }

    /// Point the trail at a new cursor rect (pixels). Returns true if an animation started.
    pub fn set_target(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let t = quad(x, y, w, h);
        if t == self.target {
            return false;
        }
        let (a, b) = (center(&self.target), center(&t));
        let dist = ((b[0] - a[0]) / w).hypot((b[1] - a[1]) / h);
        self.target = t;
        let first = self.target == [[0.0; 2]; 4];
        if !self.active && (dist < START_THRESHOLD_CELLS || first) {
            self.corners = t;
            return false;
        }
        self.active = true;
        true
    }

    /// Jump straight to the target (trail disabled).
    pub fn snap(&mut self) {
        self.corners = self.target;
        self.active = false;
    }

    /// Advance by `dt` seconds. Returns true while still animating.
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.active {
            return false;
        }
        let (tc, cc) = (center(&self.target), center(&self.corners));
        let (mx, my) = (tc[0] - cc[0], tc[1] - cc[1]);
        let mlen = mx.hypot(my).max(1e-3);
        let mut done = true;
        for i in 0..4 {
            let (t, c) = (self.target[i], &mut self.corners[i]);
            // How much this corner points along the direction of travel: 1 leading, -1 trailing.
            let (ox, oy) = (t[0] - tc[0], t[1] - tc[1]);
            let dot = (ox * mx + oy * my) / (ox.hypot(oy).max(1e-3) * mlen);
            // Sharp blend: anything clearly ahead is fast, clearly behind is slow.
            let lead = (dot * 4.0).clamp(-1.0, 1.0);
            let decay = DECAY_SLOW + (DECAY_FAST - DECAY_SLOW) * (lead + 1.0) * 0.5;
            // After `decay` seconds 99% of the distance is covered.
            let k = 1.0 - 0.01f32.powf(dt / decay);
            c[0] += (t[0] - c[0]) * k;
            c[1] += (t[1] - c[1]) * k;
            if (t[0] - c[0]).abs() > 0.5 || (t[1] - c[1]).abs() > 0.5 {
                done = false;
            }
        }
        if done {
            self.corners = self.target;
            self.active = false;
        }
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_moves_snap_long_moves_trail_and_converge() {
        let mut t = Trail::new();
        t.set_target(0.0, 0.0, 10.0, 20.0);
        assert!(!t.set_target(10.0, 0.0, 10.0, 20.0), "one-cell move must snap");
        assert!(t.set_target(500.0, 0.0, 10.0, 20.0), "long jump must animate");

        t.step(1.0 / 60.0);
        // Moving right: right-edge corners (tr, br) lead, left-edge corners (tl, bl) trail.
        let lead = t.corners[1][0] - 20.0; // tr started at x=20
        let trail = t.corners[0][0] - 10.0; // tl started at x=10
        assert!(lead > trail * 2.0, "leading corner must move faster");

        let mut frames = 0;
        while t.step(1.0 / 60.0) {
            frames += 1;
            assert!(frames < 120, "trail must settle within 2s");
        }
        assert_eq!(t.corners, quad(500.0, 0.0, 10.0, 20.0));
    }
}
