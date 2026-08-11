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

/// How much further the centre reaches than an edge when snapping.
pub const CENTRE_SNAP_FACTOR: f64 = 2.0;

/// The photo the window is being framed against, and how eagerly the window
/// sticks to it.
///
/// Two rules come out of this, both of them about not letting the window drift
/// somewhere useless:
///
/// * It may not grow past the *cover* size — the smallest window of the chosen
///   proportion that still contains the whole photo. Beyond that the window is
///   bigger than the picture in both directions at once, so every extra pixel
///   is border and none of it is photograph.
/// * Its centre stays inside the photo, so it can be pushed well off to one
///   side for a deliberate margin but cannot be dragged off into nothing.
///
/// Within those, edges and the two natural sizes are magnetic: come within
/// `snap` of them and the window lands exactly on them, which is what makes a
/// flush edge or an exact fit reachable by hand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraints {
    pub image_w: f64,
    pub image_h: f64,
    /// Snap distance in image pixels. Zero switches snapping off.
    pub snap: f64,
}

impl Constraints {
    pub fn new(image_w: f64, image_h: f64, snap: f64) -> Constraints {
        Constraints {
            image_w: image_w.max(1.0),
            image_h: image_h.max(1.0),
            snap: snap.max(0.0),
        }
    }

    /// Largest window worth having: the whole photo just fits inside it.
    pub fn cover_size(&self, ratio: f64) -> (f64, f64) {
        let ratio = sane_ratio(ratio);
        if self.image_w / self.image_h > ratio {
            (self.image_w, self.image_w / ratio)
        } else {
            (self.image_h * ratio, self.image_h)
        }
    }

    /// The starting size: the window just fits inside the photo.
    pub fn contain_size(&self, ratio: f64) -> (f64, f64) {
        let b = CropBox::fit_centered(self.image_w, self.image_h, ratio);
        (b.w, b.h)
    }
}

fn sane_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    }
}

/// Which point of the box a gesture holds still, as a fraction of its size.
/// Dragging the bottom-right corner pins the top-left, and so on; moving pins
/// the centre.
fn anchor_fraction(handle: Handle) -> (f64, f64) {
    match handle {
        Handle::TopLeft => (1.0, 1.0),
        Handle::Top => (0.5, 1.0),
        Handle::TopRight => (0.0, 1.0),
        Handle::Right => (0.0, 0.5),
        Handle::BottomRight => (0.0, 0.0),
        Handle::Bottom => (0.5, 0.0),
        Handle::BottomLeft => (1.0, 0.0),
        Handle::Left => (1.0, 0.5),
        Handle::Move => (0.5, 0.5),
    }
}

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

    /// Resizes to `new_w`, holding `ratio`, keeping the point at the given
    /// fraction of the box fixed.
    pub fn resized_to(&self, new_w: f64, ratio: f64, anchor: (f64, f64)) -> CropBox {
        let ratio = sane_ratio(ratio);
        let w = new_w.max(MIN_SIZE);
        let h = (w / ratio).max(MIN_SIZE);
        // Recover the width if the height was the binding minimum, so the
        // proportion survives even at the very bottom of the range.
        let w = h * ratio;
        CropBox {
            x: self.x + (self.w - w) * anchor.0,
            y: self.y + (self.h - h) * anchor.1,
            w,
            h,
        }
    }

    /// Nudges each axis onto the image edge or the centre line when it is
    /// already close, so a flush edge is reachable by hand.
    ///
    /// The centre pulls harder than the edges — see `CENTRE_SNAP_FACTOR` — and
    /// wins outright when both are in range. Dead centre is the framing people
    /// reach for most and the one that is most obvious when it is a few pixels
    /// out, so it gets the benefit of the doubt.
    pub fn snap_position(&self, c: Constraints) -> CropBox {
        if c.snap <= 0.0 {
            return *self;
        }
        let snap_axis = |pos: f64, size: f64, extent: f64| -> f64 {
            let centre = (extent - size) / 2.0;
            if (centre - pos).abs() <= c.snap * CENTRE_SNAP_FACTOR {
                return centre;
            }
            [0.0, extent - size]
                .into_iter()
                .filter(|candidate| (candidate - pos).abs() <= c.snap)
                .min_by(|a, b| (a - pos).abs().total_cmp(&(b - pos).abs()))
                .unwrap_or(pos)
        };
        CropBox {
            x: snap_axis(self.x, self.w, c.image_w),
            y: snap_axis(self.y, self.h, c.image_h),
            ..*self
        }
    }

    /// Keeps the centre of the window inside the photo.
    pub fn clamp_position(&self, c: Constraints) -> CropBox {
        let (cx, cy) = self.center();
        let clamped_x = cx.clamp(0.0, c.image_w);
        let clamped_y = cy.clamp(0.0, c.image_h);
        CropBox {
            x: self.x + (clamped_x - cx),
            y: self.y + (clamped_y - cy),
            ..*self
        }
    }

    /// A drag of the whole window: move, stick to edges, stay on the photo.
    pub fn apply_move(&self, dx: f64, dy: f64, c: Constraints) -> CropBox {
        self.translated(dx, dy).snap_position(c).clamp_position(c)
    }

    /// A drag of one handle: resize on the locked proportion, refusing to grow
    /// past the cover size and sticking to the sizes that line up with the
    /// photo's edges.
    pub fn apply_drag(
        &self,
        handle: Handle,
        pointer: (f64, f64),
        ratio: f64,
        c: Constraints,
    ) -> CropBox {
        if handle == Handle::Move {
            return *self;
        }
        let ratio = sane_ratio(ratio);
        let raw = self.dragged(handle, pointer, ratio);
        let (cover_w, _) = c.cover_size(ratio);
        let anchor = anchor_fraction(handle);

        let mut width = raw.w.clamp(MIN_SIZE, cover_w.max(MIN_SIZE));
        if let Some(snapped) = nearest_within(width, &size_candidates(&raw, handle, ratio, c), c.snap)
        {
            width = snapped.clamp(MIN_SIZE, cover_w.max(MIN_SIZE));
        }

        raw.resized_to(width, ratio, anchor).clamp_position(c)
    }

    /// The scroll wheel: zoom about the centre, within the same limits.
    pub fn apply_zoom(&self, factor: f64, ratio: f64, c: Constraints) -> CropBox {
        let ratio = sane_ratio(ratio);
        let factor = if factor.is_finite() && factor > 0.0 {
            factor
        } else {
            1.0
        };
        let (cover_w, _) = c.cover_size(ratio);
        let (contain_w, _) = c.contain_size(ratio);

        let mut width = (self.w * factor).clamp(MIN_SIZE, cover_w.max(MIN_SIZE));
        if let Some(snapped) = nearest_within(width, &[contain_w, cover_w], c.snap) {
            width = snapped.clamp(MIN_SIZE, cover_w.max(MIN_SIZE));
        }

        self.resized_to(width, ratio, (0.5, 0.5))
            .snap_position(c)
            .clamp_position(c)
    }

    /// Rounded to whole pixels for the actual export. Width and height are
    /// rounded first so the printed proportions survive the rounding.
    pub fn to_pixel_rect(&self) -> (i64, i64, u32, u32) {
        let w = self.w.round().max(1.0) as u32;
        let h = self.h.round().max(1.0) as u32;
        (self.x.round() as i64, self.y.round() as i64, w, h)
    }
}

/// Widths the drag should stick to: the two natural sizes, plus whichever
/// width puts the free edge exactly on the photo's edge.
fn size_candidates(b: &CropBox, handle: Handle, ratio: f64, c: Constraints) -> Vec<f64> {
    let (contain_w, _) = c.contain_size(ratio);
    let (cover_w, _) = c.cover_size(ratio);
    let mut out = vec![contain_w, cover_w];

    let (ax, ay) = anchor_fraction(handle);
    // A fraction of 0 means that edge is pinned and the far one is free.
    if ax == 0.0 {
        out.push(c.image_w - b.x);
    } else if ax == 1.0 {
        out.push(b.right());
    }
    if ay == 0.0 {
        out.push((c.image_h - b.y) * ratio);
    } else if ay == 1.0 {
        out.push(b.bottom() * ratio);
    }

    out.retain(|w| w.is_finite() && *w >= MIN_SIZE);
    out
}

/// The closest candidate to `value`, if any is within `tolerance`.
fn nearest_within(value: f64, candidates: &[f64], tolerance: f64) -> Option<f64> {
    if tolerance <= 0.0 {
        return None;
    }
    candidates
        .iter()
        .copied()
        .filter(|candidate| (candidate - value).abs() <= tolerance)
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
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

    // ---- constrained gestures -------------------------------------------

    /// 4000x3000 photo, 10x15 portrait window (ratio 2/3), 20 px of magnetism.
    fn constraints() -> Constraints {
        Constraints::new(4000.0, 3000.0, 20.0)
    }

    const PORTRAIT: f64 = 2.0 / 3.0;

    #[test]
    fn cover_is_the_smallest_window_containing_the_whole_photo() {
        let c = constraints();
        let (w, h) = c.cover_size(PORTRAIT);
        assert!((w - 4000.0).abs() < EPS);
        assert!((h - 6000.0).abs() < EPS);
        assert!(w >= c.image_w && h >= c.image_h, "must cover the photo");

        // Landscape target on the same photo binds on the other axis.
        let (w, h) = c.cover_size(1.5);
        assert!((w - 4500.0).abs() < EPS);
        assert!((h - 3000.0).abs() < EPS);
    }

    #[test]
    fn the_window_cannot_grow_past_the_photo_in_both_directions() {
        let c = constraints();
        let start = CropBox::fit_centered(4000.0, 3000.0, PORTRAIT);
        // Haul the bottom-right handle far beyond everything.
        let out = start.apply_drag(Handle::BottomRight, (99_000.0, 99_000.0), PORTRAIT, c);

        let (cover_w, cover_h) = c.cover_size(PORTRAIT);
        assert!(out.w <= cover_w + EPS, "{} > {cover_w}", out.w);
        assert!(out.h <= cover_h + EPS, "{} > {cover_h}", out.h);
        assert_ratio(out, PORTRAIT);
        // At the limit one dimension still matches the photo exactly, so there
        // is never a border on all four sides at once.
        assert!(
            out.w <= c.image_w + EPS || out.h <= c.image_h + EPS,
            "border on every side: {out:?}"
        );
    }

    #[test]
    fn zooming_out_stops_at_the_cover_size() {
        let c = constraints();
        let start = CropBox::fit_centered(4000.0, 3000.0, PORTRAIT);
        let mut b = start;
        for _ in 0..50 {
            b = b.apply_zoom(1.3, PORTRAIT, c);
        }
        let (cover_w, _) = c.cover_size(PORTRAIT);
        assert!((b.w - cover_w).abs() < EPS, "settled at {} not {cover_w}", b.w);
        assert_ratio(b, PORTRAIT);
    }

    #[test]
    fn zooming_in_stops_at_the_minimum() {
        let c = constraints();
        let mut b = CropBox::fit_centered(4000.0, 3000.0, PORTRAIT);
        for _ in 0..200 {
            b = b.apply_zoom(0.7, PORTRAIT, c);
        }
        assert!(b.w >= MIN_SIZE - EPS && b.h >= MIN_SIZE - EPS, "{b:?}");
        assert_ratio(b, PORTRAIT);
    }

    #[test]
    fn moving_sticks_to_the_left_edge() {
        let c = constraints();
        let start = CropBox::new(12.0, 400.0, 2000.0, 3000.0);
        // A move that would leave x at 3 lands it exactly on 0.
        let out = start.apply_move(-9.0, 0.0, c);
        assert!(out.x.abs() < EPS, "x = {}", out.x);
        assert!((out.w - 2000.0).abs() < EPS, "size must not change");
    }

    #[test]
    fn moving_sticks_to_the_right_edge_and_the_centre_line() {
        let c = constraints();
        let start = CropBox::new(1985.0, 0.0, 2000.0, 3000.0);
        let out = start.apply_move(5.0, 0.0, c);
        assert!((out.right() - 4000.0).abs() < EPS, "right = {}", out.right());

        let centred = CropBox::new(990.0, 0.0, 2000.0, 3000.0);
        let out = centred.apply_move(5.0, 0.0, c);
        assert!((out.x - 1000.0).abs() < EPS, "x = {}", out.x);
    }

    #[test]
    fn the_centre_pulls_harder_than_an_edge() {
        let c = constraints();
        // Centred x for a 2000-wide window on a 4000-wide photo is 1000.
        let centred = 1000.0;

        // Beyond the plain snap distance but inside the centre's wider reach.
        let offset = c.snap * 1.5;
        let start = CropBox::new(centred + offset, 0.0, 2000.0, 3000.0);
        let out = start.apply_move(0.0, 0.0, c);
        assert!((out.x - centred).abs() < EPS, "x = {} should be centred", out.x);

        // Well past even that, it is left where it is.
        let far = CropBox::new(centred + c.snap * CENTRE_SNAP_FACTOR + 30.0, 0.0, 2000.0, 3000.0);
        let out = far.apply_move(0.0, 0.0, c);
        assert!((out.x - far.x).abs() < EPS, "x = {} should not have moved", out.x);
    }

    #[test]
    fn the_centre_wins_when_an_edge_is_also_in_range() {
        // A window almost as wide as the photo puts the centre and both edges
        // within a few pixels of each other; the centre should take it.
        let c = Constraints::new(1000.0, 1000.0, 30.0);
        let start = CropBox::new(6.0, 0.0, 990.0, 990.0);
        let out = start.apply_move(0.0, 0.0, c);
        assert!((out.x - 5.0).abs() < EPS, "x = {} should be the centre, 5", out.x);
    }

    #[test]
    fn both_axes_stick_to_the_centre_independently() {
        let c = constraints();
        let start = CropBox::new(1000.0 + 10.0, 0.0 - 8.0, 2000.0, 3000.0);
        let out = start.apply_move(0.0, 0.0, c);
        // Centred vertically for a 3000-tall window on a 3000-tall photo is 0.
        assert!((out.x - 1000.0).abs() < EPS);
        assert!(out.y.abs() < EPS);
    }

    #[test]
    fn a_slow_drag_can_leave_a_snapped_edge() {
        let c = constraints();
        let start = CropBox::new(0.0, 0.0, 2000.0, 3000.0); // flush against the left

        // Twenty small steps, each on its own well inside the snap radius, but
        // always measured from where the drag began. The window has to come
        // away from the edge.
        let mut out = start;
        for step in 1..=20 {
            out = start.apply_move(step as f64 * 5.0, 0.0, c);
        }
        assert!(out.x > c.snap, "never left the edge: x = {}", out.x);
    }

    /// Why the editor measures a move from where the drag began rather than
    /// adding each frame's delta to the current position: feeding small steps
    /// back into an already-snapped box means every one of them is undone by the
    /// snap that follows, and the window is glued to the edge for good.
    #[test]
    fn feeding_small_steps_back_into_a_snapped_box_never_escapes() {
        let c = constraints();
        let mut stuck = CropBox::new(0.0, 0.0, 2000.0, 3000.0);
        for _ in 0..20 {
            stuck = stuck.apply_move(5.0, 0.0, c);
        }
        assert!(stuck.x.abs() < EPS, "this is the trap, x = {}", stuck.x);
    }

    #[test]
    fn an_oversized_window_still_sticks_to_the_centre() {
        let c = constraints();
        // The cover window is taller than the photo, so its centred position is
        // above the top edge — the case where sticking to the sides instead of
        // the centre makes it impossible to place.
        let (w, h) = c.cover_size(PORTRAIT);
        let centred_y = (c.image_h - h) / 2.0;
        assert!(centred_y < 0.0, "this test is about an oversized window");

        let nudged = CropBox::new(0.0, centred_y + c.snap, w, h);
        let out = nudged.apply_move(0.0, 0.0, c);
        assert!(
            (out.y - centred_y).abs() < EPS,
            "y = {} should have settled on the centre {centred_y}",
            out.y
        );

        // And it is still possible to get away from that centre deliberately.
        let away = nudged.apply_move(0.0, c.snap * CENTRE_SNAP_FACTOR + 40.0, c);
        assert!((away.y - centred_y).abs() > c.snap, "y = {} is still glued", away.y);
    }

    #[test]
    fn a_move_well_clear_of_an_edge_is_left_alone() {
        let c = constraints();
        let start = CropBox::new(1000.0, 0.0, 2000.0, 3000.0);
        let out = start.apply_move(-300.0, 0.0, c);
        assert!((out.x - 700.0).abs() < EPS, "x = {}", out.x);
    }

    #[test]
    fn snapping_can_be_switched_off() {
        let loose = Constraints::new(4000.0, 3000.0, 0.0);
        let start = CropBox::new(12.0, 400.0, 2000.0, 3000.0);
        let out = start.apply_move(-9.0, 0.0, loose);
        assert!((out.x - 3.0).abs() < EPS, "x = {}", out.x);
    }

    #[test]
    fn the_window_cannot_be_dragged_off_the_photo() {
        let c = constraints();
        let start = CropBox::fit_centered(4000.0, 3000.0, PORTRAIT);
        let out = start.apply_move(-50_000.0, -50_000.0, c);
        let (cx, cy) = out.center();
        assert!((0.0..=4000.0).contains(&cx), "centre x escaped: {cx}");
        assert!((0.0..=3000.0).contains(&cy), "centre y escaped: {cy}");
        // It is still allowed well outside — that is the printed border.
        assert!(out.overflows(4000.0, 3000.0));
    }

    #[test]
    fn a_resize_sticks_to_the_size_that_lines_the_free_edge_up_with_the_photo() {
        let c = constraints();
        // Left edge pinned at 0; dragging the right edge to just short of the
        // photo's right edge should land it exactly there.
        let start = CropBox::new(0.0, 0.0, 1000.0, 1500.0);
        let out = start.apply_drag(Handle::Right, (3990.0, 750.0), PORTRAIT, c);
        assert!((out.right() - 4000.0).abs() < EPS, "right = {}", out.right());
        assert!(out.x.abs() < EPS, "the pinned edge moved");
        assert_ratio(out, PORTRAIT);
    }

    #[test]
    fn a_resize_sticks_to_the_in_fit_size() {
        let c = constraints();
        let (contain_w, _) = c.contain_size(PORTRAIT);
        let start = CropBox::new(1000.0, 0.0, contain_w - 60.0, (contain_w - 60.0) / PORTRAIT);
        // Drag the corner out to within the snap distance of the fit size.
        let target = start.x + contain_w - 8.0;
        let out = start.apply_drag(Handle::BottomRight, (target, start.y + 10.0), PORTRAIT, c);
        assert!((out.w - contain_w).abs() < EPS, "w = {} not {contain_w}", out.w);
    }

    #[test]
    fn a_resize_holds_its_anchor() {
        let c = constraints();
        let start = CropBox::new(500.0, 400.0, 1200.0, 1800.0);
        let out = start.apply_drag(Handle::TopLeft, (300.0, 100.0), PORTRAIT, c);
        assert!((out.right() - start.right()).abs() < EPS, "anchor drifted");
        assert!((out.bottom() - start.bottom()).abs() < EPS, "anchor drifted");
        assert_ratio(out, PORTRAIT);
    }

    #[test]
    fn dragging_the_move_handle_through_apply_drag_does_nothing() {
        let c = constraints();
        let start = CropBox::new(500.0, 400.0, 1200.0, 1800.0);
        assert_eq!(start.apply_drag(Handle::Move, (0.0, 0.0), PORTRAIT, c), start);
    }

    #[test]
    fn constrained_gestures_never_break_the_proportion() {
        let c = constraints();
        let mut b = CropBox::fit_centered(4000.0, 3000.0, PORTRAIT);
        let handles = [
            Handle::TopLeft,
            Handle::Top,
            Handle::TopRight,
            Handle::Right,
            Handle::BottomRight,
            Handle::Bottom,
            Handle::BottomLeft,
            Handle::Left,
        ];
        // A long, deliberately hostile sequence of gestures.
        for (i, handle) in handles.iter().cycle().take(120).enumerate() {
            let x = ((i * 977) % 9000) as f64 - 2500.0;
            let y = ((i * 613) % 7000) as f64 - 2000.0;
            b = b.apply_drag(*handle, (x, y), PORTRAIT, c);
            b = b.apply_move(x / 10.0, y / 10.0, c);
            b = b.apply_zoom(if i % 3 == 0 { 1.2 } else { 0.85 }, PORTRAIT, c);

            assert_ratio(b, PORTRAIT);
            assert!(b.w >= MIN_SIZE - EPS && b.h >= MIN_SIZE - EPS, "collapsed: {b:?}");
            let (cover_w, cover_h) = c.cover_size(PORTRAIT);
            assert!(b.w <= cover_w + EPS && b.h <= cover_h + EPS, "overgrew: {b:?}");
            let (cx, cy) = b.center();
            assert!((0.0..=4000.0).contains(&cx) && (0.0..=3000.0).contains(&cy), "escaped: {b:?}");
        }
    }

    #[test]
    fn pixel_rect_rounds_consistently() {
        let b = CropBox::new(10.4, 20.6, 100.5, 67.0);
        let (x, y, w, h) = b.to_pixel_rect();
        assert_eq!((x, y), (10, 21));
        assert_eq!((w, h), (101, 67));
    }
}
