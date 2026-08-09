//! The cut-out window.
//!
//! Coordinates are source-image pixels, with the origin at the top left and the
//! image already rotated upright. The box is deliberately *not* clamped to the
//! image: the brief calls for a window that may be larger than the photo, which
//! is how you get a deliberate white border or an off-centre composition with
//! breathing room. Whatever falls outside the source is filled with the
//! configured background colour at export time.
//!
//! The one invariant that is always enforced is the aspect ratio.

/// What the pointer is grabbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Move,
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const CORNERS: [Handle; 4] = [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
    ];
    pub const EDGES: [Handle; 4] = [Handle::Top, Handle::Right, Handle::Bottom, Handle::Left];

    fn is_corner(self) -> bool {
        Handle::CORNERS.contains(&self)
    }
}

/// Smallest crop the box may be shrunk to, in source pixels. Below this the
/// handles become impossible to grab and the export is pointless anyway.
pub const MIN_SIZE: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl CropBox {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> CropBox {
        CropBox { x, y, w, h }
    }

    /// The starting position: the largest box of `ratio` that fits entirely
    /// inside the image, centred. `ratio` is width / height.
    pub fn fit_centered(image_w: f64, image_h: f64, ratio: f64) -> CropBox {
        let ratio = if ratio.is_finite() && ratio > 0.0 {
            ratio
        } else {
            1.0
        };
        let (w, h) = if image_w / image_h > ratio {
            // Image is wider than the target: height is the binding side.
            (image_h * ratio, image_h)
        } else {
            (image_w, image_w / ratio)
        };
        CropBox {
            x: (image_w - w) / 2.0,
            y: (image_h - h) / 2.0,
            w,
            h,
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn ratio(&self) -> f64 {
        if self.h <= 0.0 {
            1.0
        } else {
            self.w / self.h
        }
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.right() && py >= self.y && py <= self.bottom()
    }

    /// True when the window sticks out past the image, i.e. the export will
    /// contain background fill. The UI warns about this.
    pub fn overflows(&self, image_w: f64, image_h: f64) -> bool {
        self.x < -0.5 || self.y < -0.5 || self.right() > image_w + 0.5 || self.bottom() > image_h + 0.5
    }

    pub fn translated(&self, dx: f64, dy: f64) -> CropBox {
        CropBox {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// Zooms about the centre, e.g. from the scroll wheel.
    pub fn scaled_about_center(&self, factor: f64) -> CropBox {
        let (cx, cy) = self.center();
        let factor = if factor.is_finite() && factor > 0.0 {
            factor
        } else {
            1.0
        };
        let w = (self.w * factor).max(MIN_SIZE);
        let h = (self.h * factor).max(MIN_SIZE * self.h / self.w.max(f64::EPSILON));
        CropBox {
            x: cx - w / 2.0,
            y: cy - h / 2.0,
            w,
            h,
        }
    }

    /// Re-proportions the box to a new ratio, keeping the centre and roughly
    /// the same area, so switching 10×15 → 13×18 does not jump the framing.
    pub fn with_ratio(&self, ratio: f64) -> CropBox {
        let ratio = if ratio.is_finite() && ratio > 0.0 {
            ratio
        } else {
            1.0
        };
        let (cx, cy) = self.center();
        let area = (self.w * self.h).max(MIN_SIZE * MIN_SIZE);
        let h = (area / ratio).sqrt();
        let w = h * ratio;
        CropBox {
            x: cx - w / 2.0,
            y: cy - h / 2.0,
            w,
            h,
        }
    }

    /// Applies a pointer drag on `handle` to the box, holding `ratio`.
    ///
    /// `pointer` is the current pointer position in image pixels. Corner drags
    /// anchor the opposite corner; edge drags anchor the opposite edge and grow
    /// symmetrically along the other axis, which is the least surprising
    /// behaviour once the ratio is locked.
    pub fn dragged(&self, handle: Handle, pointer: (f64, f64), ratio: f64) -> CropBox {
        let ratio = if ratio.is_finite() && ratio > 0.0 {
            ratio
        } else {
            self.ratio()
        };
        let (px, py) = pointer;

        match handle {
            Handle::Move => *self,

            h if h.is_corner() => {
                let (ax, ay) = match h {
                    Handle::TopLeft => (self.right(), self.bottom()),
                    Handle::TopRight => (self.x, self.bottom()),
                    Handle::BottomRight => (self.x, self.y),
                    Handle::BottomLeft => (self.right(), self.y),
                    _ => unreachable!(),
                };
                let dx = px - ax;
                let dy = py - ay;
                // Follow whichever axis the pointer pushed further, so the box
                // tracks diagonal movement instead of ignoring one direction.
                let mut w = dx.abs().max(dy.abs() * ratio);
                if w < MIN_SIZE {
                    w = MIN_SIZE;
                }
                let hgt = w / ratio;
                // Keep growing away from the anchor in the direction the
                // handle already lay, so the box cannot flip inside out.
                let sx = if dx == 0.0 {
                    default_sign_x(h)
                } else {
                    dx.signum()
                };
                let sy = if dy == 0.0 {
                    default_sign_y(h)
                } else {
                    dy.signum()
                };
                let x0 = if sx >= 0.0 { ax } else { ax - w };
                let y0 = if sy >= 0.0 { ay } else { ay - hgt };
                CropBox {
                    x: x0,
                    y: y0,
                    w,
                    h: hgt,
                }
            }

            Handle::Top | Handle::Bottom => {
                let anchor_y = if handle == Handle::Top {
                    self.bottom()
                } else {
                    self.y
                };
                let hgt = (py - anchor_y).abs().max(MIN_SIZE / ratio.max(f64::EPSILON)).max(MIN_SIZE);
                let w = hgt * ratio;
                let (cx, _) = self.center();
                let y0 = if handle == Handle::Top {
                    anchor_y - hgt
                } else {
                    anchor_y
                };
                CropBox {
                    x: cx - w / 2.0,
                    y: y0,
                    w,
                    h: hgt,
                }
            }

            Handle::Left | Handle::Right => {
                let anchor_x = if handle == Handle::Left {
                    self.right()
                } else {
                    self.x
                };
                let w = (px - anchor_x).abs().max(MIN_SIZE);
                let hgt = w / ratio;
                let (_, cy) = self.center();
                let x0 = if handle == Handle::Left {
                    anchor_x - w
                } else {
                    anchor_x
                };
                CropBox {
                    x: x0,
                    y: cy - hgt / 2.0,
                    w,
                    h: hgt,
                }
            }

            _ => *self,
        }
    }

    /// Which handle sits under a point, `tolerance` being the grab radius in
    /// image pixels. Corners win over edges, and the interior is `Move`.
    pub fn hit_test(&self, px: f64, py: f64, tolerance: f64) -> Option<Handle> {
        let near = |a: f64, b: f64| (a - b).abs() <= tolerance;
        let within_x = px >= self.x - tolerance && px <= self.right() + tolerance;
        let within_y = py >= self.y - tolerance && py <= self.bottom() + tolerance;

        if within_x && within_y {
            let left = near(px, self.x);
            let right = near(px, self.right());
            let top = near(py, self.y);
            let bottom = near(py, self.bottom());
            match (left, right, top, bottom) {
                (true, _, true, _) => return Some(Handle::TopLeft),
                (_, true, true, _) => return Some(Handle::TopRight),
                (_, true, _, true) => return Some(Handle::BottomRight),
                (true, _, _, true) => return Some(Handle::BottomLeft),
                (true, ..) => return Some(Handle::Left),
                (_, true, ..) => return Some(Handle::Right),
                (_, _, true, _) => return Some(Handle::Top),
                (_, _, _, true) => return Some(Handle::Bottom),
                _ => {}
            }
        }

        self.contains(px, py).then_some(Handle::Move)
    }

    /// Rounded to whole pixels for the actual export. Width and height are
    /// rounded first so the printed proportions survive the rounding.
    pub fn to_pixel_rect(&self) -> (i64, i64, u32, u32) {
        let w = self.w.round().max(1.0) as u32;
        let h = self.h.round().max(1.0) as u32;
        (self.x.round() as i64, self.y.round() as i64, w, h)
    }
}

fn default_sign_x(h: Handle) -> f64 {
    match h {
        Handle::TopLeft | Handle::BottomLeft => -1.0,
        _ => 1.0,
    }
}

fn default_sign_y(h: Handle) -> f64 {
    match h {
        Handle::TopLeft | Handle::TopRight => -1.0,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn assert_ratio(b: CropBox, ratio: f64) {
        assert!(
            (b.ratio() - ratio).abs() < 1e-6,
            "ratio drifted: {} vs {ratio} ({b:?})",
            b.ratio()
        );
    }

    #[test]
    fn starts_in_fit_and_centered_on_a_landscape_image() {
        // 4000x3000 photo, 10:15 portrait target (ratio 2/3).
        let b = CropBox::fit_centered(4000.0, 3000.0, 2.0 / 3.0);
        assert!((b.h - 3000.0).abs() < EPS);
        assert!((b.w - 2000.0).abs() < EPS);
        assert!((b.x - 1000.0).abs() < EPS);
        assert!(b.y.abs() < EPS);
        assert!(!b.overflows(4000.0, 3000.0));
        assert_ratio(b, 2.0 / 3.0);
    }

    #[test]
    fn starts_in_fit_and_centered_on_a_portrait_image() {
        let b = CropBox::fit_centered(3000.0, 4000.0, 1.5);
        assert!((b.w - 3000.0).abs() < EPS);
        assert!((b.h - 2000.0).abs() < EPS);
        assert!(b.x.abs() < EPS);
        assert!((b.y - 1000.0).abs() < EPS);
    }

    #[test]
    fn a_square_target_on_a_square_image_fills_it() {
        let b = CropBox::fit_centered(1000.0, 1000.0, 1.0);
        assert_eq!(b, CropBox::new(0.0, 0.0, 1000.0, 1000.0));
    }

    #[test]
    fn moving_does_not_change_size_and_may_leave_the_image() {
        let b = CropBox::fit_centered(4000.0, 3000.0, 2.0 / 3.0).translated(-1500.0, -400.0);
        assert!((b.w - 2000.0).abs() < EPS);
        assert!(b.x < 0.0);
        assert!(b.overflows(4000.0, 3000.0), "window is allowed outside the image");
    }

    #[test]
    fn corner_drag_holds_the_ratio_and_anchors_the_opposite_corner() {
        let b = CropBox::new(100.0, 100.0, 600.0, 400.0); // ratio 1.5
        let d = b.dragged(Handle::BottomRight, (1000.0, 120.0), 1.5);
        // Top-left corner stayed put.
        assert!((d.x - 100.0).abs() < EPS);
        assert!((d.y - 100.0).abs() < EPS);
        assert_ratio(d, 1.5);
        assert!((d.w - 900.0).abs() < EPS);
    }

    #[test]
    fn corner_drag_follows_the_dominant_axis() {
        let b = CropBox::new(0.0, 0.0, 300.0, 200.0);
        // Pointer barely moved horizontally but a long way vertically.
        let d = b.dragged(Handle::BottomRight, (310.0, 600.0), 1.5);
        assert_ratio(d, 1.5);
        assert!(d.h >= 600.0 - EPS, "vertical drag was ignored: {d:?}");
    }

    #[test]
    fn corner_drag_past_the_anchor_flips_the_box_without_inverting_it() {
        let b = CropBox::new(100.0, 100.0, 600.0, 400.0);
        // Drag the bottom-right handle above and left of the anchor.
        let d = b.dragged(Handle::BottomRight, (-200.0, -100.0), 1.5);
        assert!(d.w > 0.0 && d.h > 0.0, "size went negative: {d:?}");
        assert_ratio(d, 1.5);
        assert!(d.right() <= 100.0 + EPS, "should sit left of the anchor");
    }

    #[test]
    fn edge_drag_keeps_the_ratio_and_stays_centered_on_the_other_axis() {
        let b = CropBox::new(100.0, 100.0, 600.0, 400.0);
        let before_cx = b.center().0;
        let d = b.dragged(Handle::Bottom, (0.0, 900.0), 1.5);
        assert_ratio(d, 1.5);
        assert!((d.y - 100.0).abs() < EPS, "top edge is the anchor");
        assert!((d.h - 800.0).abs() < EPS);
        assert!((d.center().0 - before_cx).abs() < EPS);
    }

    #[test]
    fn drags_never_shrink_below_the_minimum() {
        let b = CropBox::new(0.0, 0.0, 600.0, 400.0);
        let d = b.dragged(Handle::BottomRight, (0.0, 0.0), 1.5);
        assert!(d.w >= MIN_SIZE - EPS && d.h > 0.0);
        assert_ratio(d, 1.5);
    }

    #[test]
    fn scaling_about_the_center_keeps_the_center() {
        let b = CropBox::new(100.0, 200.0, 600.0, 400.0);
        let before = b.center();
        let d = b.scaled_about_center(1.25);
        assert!((d.center().0 - before.0).abs() < EPS);
        assert!((d.center().1 - before.1).abs() < EPS);
        assert!((d.w - 750.0).abs() < EPS);
        assert_ratio(d, 1.5);
    }

    #[test]
    fn changing_ratio_preserves_center_and_area() {
        let b = CropBox::new(100.0, 100.0, 600.0, 400.0);
        let before = b.center();
        let d = b.with_ratio(2.0 / 3.0);
        assert_ratio(d, 2.0 / 3.0);
        assert!((d.center().0 - before.0).abs() < EPS);
        assert!((d.center().1 - before.1).abs() < EPS);
        assert!(((d.w * d.h) - (b.w * b.h)).abs() < 1e-6);
    }

    #[test]
    fn hit_test_prefers_corners_then_edges_then_the_interior() {
        let b = CropBox::new(100.0, 100.0, 600.0, 400.0);
        assert_eq!(b.hit_test(102.0, 102.0, 10.0), Some(Handle::TopLeft));
        assert_eq!(b.hit_test(698.0, 498.0, 10.0), Some(Handle::BottomRight));
        assert_eq!(b.hit_test(400.0, 101.0, 10.0), Some(Handle::Top));
        assert_eq!(b.hit_test(699.0, 300.0, 10.0), Some(Handle::Right));
        assert_eq!(b.hit_test(400.0, 300.0, 10.0), Some(Handle::Move));
        assert_eq!(b.hit_test(20.0, 20.0, 10.0), None);
    }

    #[test]
    fn degenerate_ratios_do_not_produce_nonsense() {
        let b = CropBox::fit_centered(1000.0, 800.0, 0.0);
        assert!(b.w > 0.0 && b.h > 0.0);
        let b = CropBox::fit_centered(1000.0, 800.0, f64::NAN);
        assert!(b.w.is_finite() && b.h.is_finite());
    }

    #[test]
    fn pixel_rect_rounds_consistently() {
        let b = CropBox::new(10.4, 20.6, 100.5, 67.0);
        let (x, y, w, h) = b.to_pixel_rect();
        assert_eq!((x, y), (10, 21));
        assert_eq!((w, h), (101, 67));
    }
}
