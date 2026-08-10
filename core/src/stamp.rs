//! Burning the caption into the picture.
//!
//! The outline is not drawn by stamping the text repeatedly at offsets — that
//! is cheap to write but leaves lumpy corners and costs one full text raster
//! per offset. Instead the string is rasterised once into a coverage mask, a
//! Euclidean distance transform of that mask is taken, and the outline is the
//! band of pixels within `outline_px` of the glyph edge. That gives a smooth,
//! evenly thick outline around any shape at any size for a single raster pass.

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

use crate::config::{Color, Corner};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StampStyle {
    pub size_px: f32,
    pub fill: Color,
    pub outline: Color,
    pub outline_px: f32,
    pub margin_px: f32,
    pub corner: Corner,
}

impl StampStyle {
    /// Turns the percentage-based settings into pixels for a given crop size.
    /// Percentages are of the *short* side, so a caption keeps its apparent
    /// size whether the print is portrait or landscape.
    pub fn from_config(caption: &crate::config::CaptionConfig, crop_w: u32, crop_h: u32) -> StampStyle {
        let short = crop_w.min(crop_h).max(1) as f32;
        let size_px = (short * caption.size_pct / 100.0).max(4.0);
        StampStyle {
            size_px,
            fill: caption.fill,
            outline: caption.outline,
            outline_px: size_px * caption.outline_pct / 100.0,
            margin_px: short * caption.margin_pct / 100.0,
            corner: caption.corner,
        }
    }
}

/// Draws `text` into the corner of `target`.
///
/// Returns false when there was nothing to draw (empty text, or a font with no
/// usable glyphs for it), which the caller can treat as "no caption" rather
/// than as an error.
pub fn draw_caption(target: &mut RgbaImage, text: &str, font: &FontVec, style: &StampStyle) -> bool {
    let text = text.trim();
    if text.is_empty() || style.size_px <= 0.0 {
        return false;
    }
    let Some(mask) = rasterize(text, font, style) else {
        return false;
    };

    let radius = style.outline_px.max(0.0);
    let (img_w, img_h) = (target.width() as f32, target.height() as f32);

    // The block that has to sit inside the margin is the ink plus the outline
    // that surrounds it, otherwise a thick outline would hang off the edge.
    let visual_min_x = mask.ink_min_x - radius;
    let visual_min_y = mask.ink_min_y - radius;
    let visual_w = (mask.ink_max_x - mask.ink_min_x) + 2.0 * radius;
    let visual_h = (mask.ink_max_y - mask.ink_min_y) + 2.0 * radius;

    let dest_x = if style.corner.is_right() {
        img_w - style.margin_px - visual_w
    } else {
        style.margin_px
    };
    let dest_y = if style.corner.is_bottom() {
        img_h - style.margin_px - visual_h
    } else {
        style.margin_px
    };

    let off_x = (dest_x - visual_min_x).round() as i64;
    let off_y = (dest_y - visual_min_y).round() as i64;

    let outline_alpha = distance_outline(&mask, radius);

    for my in 0..mask.h {
        let ty = my as i64 + off_y;
        if ty < 0 || ty >= target.height() as i64 {
            continue;
        }
        for mx in 0..mask.w {
            let tx = mx as i64 + off_x;
            if tx < 0 || tx >= target.width() as i64 {
                continue;
            }
            let i = my * mask.w + mx;
            let (ox, oy) = (tx as u32, ty as u32);
            let o = outline_alpha[i];
            if o > 0.0 {
                blend(target, ox, oy, style.outline, o);
            }
            let c = mask.coverage[i];
            if c > 0.0 {
                blend(target, ox, oy, style.fill, c);
            }
        }
    }
    true
}

/// Measures the caption without drawing it, in pixels: (width, height) of the
/// ink plus outline. Used to warn when a caption would not fit.
pub fn measure(text: &str, font: &FontVec, style: &StampStyle) -> Option<(f32, f32)> {
    let mask = rasterize(text.trim(), font, style)?;
    let r = style.outline_px.max(0.0);
    Some((
        (mask.ink_max_x - mask.ink_min_x) + 2.0 * r,
        (mask.ink_max_y - mask.ink_min_y) + 2.0 * r,
    ))
}

struct Mask {
    w: usize,
    h: usize,
    coverage: Vec<f32>,
    /// Ink bounds within the mask, in mask pixel coordinates.
    ink_min_x: f32,
    ink_min_y: f32,
    ink_max_x: f32,
    ink_max_y: f32,
}

fn rasterize(text: &str, font: &FontVec, style: &StampStyle) -> Option<Mask> {
    let scale = PxScale::from(style.size_px);
    let scaled = font.as_scaled(scale);
    let line_height = scaled.height() + scaled.line_gap();
    let pad = style.outline_px.max(0.0).ceil() + 2.0;

    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return None;
    }

    // Lay every line out first so the canvas can be sized exactly once.
    let mut laid: Vec<(Vec<(GlyphId, f32)>, f32)> = Vec::with_capacity(lines.len());
    let mut widest: f32 = 0.0;
    for line in &lines {
        let mut glyphs = Vec::new();
        let mut caret = 0.0f32;
        let mut prev: Option<GlyphId> = None;
        for ch in line.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                caret += scaled.kern(p, id);
            }
            glyphs.push((id, caret));
            caret += scaled.h_advance(id);
            prev = Some(id);
        }
        widest = widest.max(caret);
        laid.push((glyphs, caret));
    }

    let canvas_w = (widest + 2.0 * pad).ceil().max(1.0) as usize;
    let canvas_h = (line_height * lines.len() as f32 + 2.0 * pad).ceil().max(1.0) as usize;
    let mut coverage = vec![0.0f32; canvas_w * canvas_h];

    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    let right_aligned = style.corner.is_right();

    for (line_index, (glyphs, line_w)) in laid.iter().enumerate() {
        // Right-hand corners get a right-aligned block, which is what reads
        // correctly when the caption sits against that edge.
        let line_offset = if right_aligned { widest - line_w } else { 0.0 };
        let baseline_y = pad + scaled.ascent() + line_height * line_index as f32;

        for (id, x) in glyphs {
            let glyph = id.with_scale_and_position(
                scale,
                ab_glyph::point(pad + line_offset + x, baseline_y),
            );
            let Some(outlined) = font.outline_glyph(glyph) else {
                continue; // whitespace and unmapped characters
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, c| {
                if c <= 0.0 {
                    return;
                }
                let px = bounds.min.x as i64 + gx as i64;
                let py = bounds.min.y as i64 + gy as i64;
                if px < 0 || py < 0 || px >= canvas_w as i64 || py >= canvas_h as i64 {
                    return;
                }
                let i = py as usize * canvas_w + px as usize;
                // Overlapping glyphs must not darken past full coverage.
                coverage[i] = coverage[i].max(c);
            });
            min_x = min_x.min(bounds.min.x);
            min_y = min_y.min(bounds.min.y);
            max_x = max_x.max(bounds.max.x);
            max_y = max_y.max(bounds.max.y);
        }
    }

    if min_x > max_x || min_y > max_y {
        return None; // nothing had an outline: all whitespace
    }

    Some(Mask {
        w: canvas_w,
        h: canvas_h,
        coverage,
        ink_min_x: min_x.max(0.0),
        ink_min_y: min_y.max(0.0),
        ink_max_x: max_x.min(canvas_w as f32),
        ink_max_y: max_y.min(canvas_h as f32),
    })
}

/// Coverage of the outline band: everything within `radius` of the glyph body.
///
/// 8SSEDT — two sweeps propagating the vector to the nearest inside pixel. The
/// result is exact enough for an outline and costs O(pixels) rather than the
/// O(pixels × radius²) of a naive dilation, which matters because the radius
/// grows with the image: a 4000 px photo puts it around 20 px.
fn distance_outline(mask: &Mask, radius: f32) -> Vec<f32> {
    let (w, h) = (mask.w, mask.h);
    if radius <= 0.0 {
        return vec![0.0; w * h];
    }

    const FAR: i32 = 1 << 14;
    let mut grid: Vec<(i32, i32)> = mask
        .coverage
        .iter()
        .map(|&c| if c > 0.5 { (0, 0) } else { (FAR, FAR) })
        .collect();

    let dist_sq = |p: (i32, i32)| (p.0 as i64).pow(2) + (p.1 as i64).pow(2);

    let compare = |grid: &mut [(i32, i32)], x: usize, y: usize, dx: i32, dy: i32| {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
            return;
        }
        let n = grid[ny as usize * w + nx as usize];
        let candidate = (n.0 - dx, n.1 - dy);
        let here = grid[y * w + x];
        if dist_sq(candidate) < dist_sq(here) {
            grid[y * w + x] = candidate;
        }
    };

    for y in 0..h {
        for x in 0..w {
            compare(&mut grid, x, y, -1, 0);
            compare(&mut grid, x, y, 0, -1);
            compare(&mut grid, x, y, -1, -1);
            compare(&mut grid, x, y, 1, -1);
        }
        for x in (0..w).rev() {
            compare(&mut grid, x, y, 1, 0);
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            compare(&mut grid, x, y, 1, 0);
            compare(&mut grid, x, y, 0, 1);
            compare(&mut grid, x, y, 1, 1);
            compare(&mut grid, x, y, -1, 1);
        }
        for x in 0..w {
            compare(&mut grid, x, y, -1, 0);
        }
    }

    grid.iter()
        .zip(&mask.coverage)
        .map(|(&p, &cov)| {
            let d = (dist_sq(p) as f32).sqrt();
            // Half a pixel of feather at the outer edge keeps it antialiased;
            // taking the max with the glyph's own coverage keeps the inner
            // edge from showing a seam where the two layers meet.
            let band = (radius + 0.5 - d).clamp(0.0, 1.0);
            band.max(cov)
        })
        .collect()
}

/// Source-over with straight (non-premultiplied) alpha on both sides.
///
/// The general form matters because the same routine draws onto an opaque photo
/// during export and onto a fully transparent canvas for the editor's caption
/// overlay. Assuming an opaque destination would multiply the edge pixels into
/// the transparent black underneath and leave the overlay with a dark fringe.
fn blend(target: &mut RgbaImage, x: u32, y: u32, color: Color, coverage: f32) {
    let sa = (coverage.clamp(0.0, 1.0) * color.a as f32) / 255.0;
    if sa <= 0.0 {
        return;
    }
    let px = target.get_pixel_mut(x, y);
    let da = px.0[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    let mix = |src: u8, dst: u8| -> u8 {
        let value = (src as f32 * sa + dst as f32 * da * (1.0 - sa)) / out_a;
        value.round().clamp(0.0, 255.0) as u8
    };
    *px = Rgba([
        mix(color.r, px.0[0]),
        mix(color.g, px.0[1]),
        mix(color.b, px.0[2]),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
}

/// A background for the settings preview: a diagonal light-to-dark ramp, so a
/// black caption with a white outline can be judged against both.
pub fn preview_backdrop(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let t = (x as f32 / w.max(1) as f32 + y as f32 / h.max(1) as f32) / 2.0;
        let v = (235.0 - 205.0 * t).round().clamp(0.0, 255.0) as u8;
        Rgba([v, v, (v as f32 * 0.98) as u8, 255])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontCatalog;

    fn test_font() -> Option<FontVec> {
        let catalog = FontCatalog::scan();
        let face = catalog.best_match("DejaVu Sans", "Bold")?;
        FontVec::try_from_vec_and_index(face.read().ok()?, face.index).ok()
    }

    fn style(corner: Corner) -> StampStyle {
        StampStyle {
            size_px: 40.0,
            fill: Color::BLACK,
            outline: Color::WHITE,
            outline_px: 5.0,
            margin_px: 20.0,
            corner,
        }
    }

    fn count_near(img: &RgbaImage, target: Color, tolerance: i32) -> usize {
        img.pixels()
            .filter(|p| {
                (p.0[0] as i32 - target.r as i32).abs() <= tolerance
                    && (p.0[1] as i32 - target.g as i32).abs() <= tolerance
                    && (p.0[2] as i32 - target.b as i32).abs() <= tolerance
            })
            .count()
    }

    /// Bounding box of pixels that differ from the untouched background.
    fn ink_bbox(img: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
        let mut bbox: Option<(u32, u32, u32, u32)> = None;
        for (x, y, p) in img.enumerate_pixels() {
            if p.0[0] == 128 && p.0[1] == 128 && p.0[2] == 128 {
                continue;
            }
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
        bbox
    }

    fn gray_canvas(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([128, 128, 128, 255]))
    }

    #[test]
    fn empty_text_draws_nothing() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(200, 100);
        assert!(!draw_caption(&mut img, "   ", &font, &style(Corner::BottomRight)));
        assert!(ink_bbox(&img).is_none());
    }

    #[test]
    fn draws_both_a_fill_and_an_outline() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(600, 200);
        assert!(draw_caption(&mut img, "Paris, France", &font, &style(Corner::BottomRight)));

        let dark = count_near(&img, Color::BLACK, 40);
        let light = count_near(&img, Color::WHITE, 40);
        assert!(dark > 200, "no fill drawn ({dark} dark pixels)");
        assert!(light > 200, "no outline drawn ({light} light pixels)");
    }

    #[test]
    fn the_outline_surrounds_the_fill_on_every_side() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(600, 200);
        draw_caption(&mut img, "H", &font, &style(Corner::TopLeft));

        let is_background = |x: i64, y: i64| {
            if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
                return true;
            }
            img.get_pixel(x as u32, y as u32).0 == [128, 128, 128, 255]
        };
        let is_light = |x: i64, y: i64| img.get_pixel(x as u32, y as u32).0[0] > 180;

        // Start from a pixel that is definitely glyph fill.
        let (fx, fy) = img
            .enumerate_pixels()
            .find(|(_, _, p)| p.0[0] < 40)
            .map(|(x, y, _)| (x as i64, y as i64))
            .expect("something was drawn");

        // Leaving the glyph in any direction must pass through the outline:
        // the last drawn pixel before the background is met has to be light.
        for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
            let (mut x, mut y) = (fx, fy);
            while !is_background(x + dx, y + dy) {
                x += dx;
                y += dy;
            }
            assert!(
                is_light(x, y),
                "going ({dx},{dy}) from the fill left the glyph without crossing the outline"
            );
        }
    }

    #[test]
    fn each_corner_places_the_caption_in_that_corner() {
        let Some(font) = test_font() else { return };
        let (w, h) = (800u32, 400u32);
        for corner in Corner::ALL {
            let mut img = gray_canvas(w, h);
            assert!(draw_caption(&mut img, "Oct '25", &font, &style(corner)));
            let (x0, y0, x1, y1) = ink_bbox(&img).expect("something was drawn");
            let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
            if corner.is_right() {
                assert!(cx > w / 2, "{corner:?} drifted left: {cx}");
            } else {
                assert!(cx < w / 2, "{corner:?} drifted right: {cx}");
            }
            if corner.is_bottom() {
                assert!(cy > h / 2, "{corner:?} drifted up: {cy}");
            } else {
                assert!(cy < h / 2, "{corner:?} drifted down: {cy}");
            }
        }
    }

    #[test]
    fn the_caption_stays_inside_the_margin() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(800, 300);
        let s = style(Corner::BottomRight);
        draw_caption(&mut img, "Moscow, Russia, Oct '25", &font, &s);
        let (x0, y0, x1, y1) = ink_bbox(&img).unwrap();
        let margin = s.margin_px as u32;
        // One pixel of slack for the rounding of the placement offset.
        assert!(x0 + 1 >= margin, "left {x0} < margin {margin}");
        assert!(y0 + 1 >= margin, "top {y0} < margin {margin}");
        assert!(x1 <= 800 - margin + 1, "right {x1} past the margin");
        assert!(y1 <= 300 - margin + 1, "bottom {y1} past the margin");
    }

    #[test]
    fn a_thicker_outline_grows_the_block() {
        let Some(font) = test_font() else { return };
        let thin = StampStyle {
            outline_px: 1.0,
            ..style(Corner::TopLeft)
        };
        let thick = StampStyle {
            outline_px: 8.0,
            ..style(Corner::TopLeft)
        };
        let (tw, th) = measure("Berlin", &font, &thin).unwrap();
        let (kw, kh) = measure("Berlin", &font, &thick).unwrap();
        assert!(kw > tw + 10.0 && kh > th + 10.0, "{kw}x{kh} vs {tw}x{th}");
    }

    #[test]
    fn zero_outline_is_allowed_and_draws_only_fill() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(400, 150);
        let s = StampStyle {
            outline_px: 0.0,
            ..style(Corner::TopLeft)
        };
        assert!(draw_caption(&mut img, "no outline", &font, &s));
        assert!(count_near(&img, Color::WHITE, 40) == 0, "outline was drawn anyway");
        assert!(count_near(&img, Color::BLACK, 40) > 100);
    }

    #[test]
    fn a_caption_larger_than_the_canvas_is_clipped_not_a_panic() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(40, 20);
        let s = StampStyle {
            size_px: 200.0,
            margin_px: 0.0,
            ..style(Corner::BottomRight)
        };
        draw_caption(&mut img, "enormous caption text", &font, &s);
    }

    #[test]
    fn multiple_lines_stack_vertically() {
        let Some(font) = test_font() else { return };
        let one = measure("A", &test_font().unwrap(), &style(Corner::TopLeft)).unwrap();
        let two = measure("A\nB", &font, &style(Corner::TopLeft)).unwrap();
        assert!(two.1 > one.1 * 1.5, "second line added no height");
    }

    #[test]
    fn cyrillic_renders() {
        let Some(font) = test_font() else { return };
        let mut img = gray_canvas(600, 200);
        assert!(draw_caption(&mut img, "Москва, Россия", &font, &style(Corner::BottomRight)));
        assert!(ink_bbox(&img).is_some());
    }

    #[test]
    fn percentages_scale_with_the_crop() {
        let caption = crate::config::CaptionConfig::default();
        let small = StampStyle::from_config(&caption, 1000, 1500);
        let large = StampStyle::from_config(&caption, 4000, 6000);
        assert!((large.size_px / small.size_px - 4.0).abs() < 0.01);
        assert!((large.margin_px / small.margin_px - 4.0).abs() < 0.01);
        // Outline is a share of the font size, so the ratio is preserved.
        assert!((small.outline_px / small.size_px - large.outline_px / large.size_px).abs() < 1e-6);
    }

    /// The editor draws the caption onto a transparent overlay. If the blend
    /// assumed an opaque destination, antialiased edges would be mixed towards
    /// transparent black and the overlay would show a grey halo.
    #[test]
    fn drawing_onto_a_transparent_canvas_leaves_no_dark_fringe() {
        let Some(font) = test_font() else { return };
        let mut img = RgbaImage::from_pixel(400, 150, Rgba([0, 0, 0, 0]));
        let s = StampStyle {
            fill: Color::WHITE,
            outline_px: 0.0,
            ..style(Corner::TopLeft)
        };
        assert!(draw_caption(&mut img, "Fringe", &font, &s));

        let touched: Vec<_> = img.pixels().filter(|p| p.0[3] > 0).collect();
        assert!(!touched.is_empty());
        for p in touched {
            assert_eq!(
                (p.0[0], p.0[1], p.0[2]),
                (255, 255, 255),
                "edge pixel was darkened: {:?}",
                p.0
            );
        }
    }

    #[test]
    fn the_preview_backdrop_has_both_light_and_dark_areas() {
        let bg = preview_backdrop(200, 100);
        let first = bg.get_pixel(0, 0).0[0];
        let last = bg.get_pixel(199, 99).0[0];
        assert!(first > last + 40, "backdrop is flat: {first} vs {last}");
    }
}
