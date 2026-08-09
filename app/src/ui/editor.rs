//! The crop editor.
//!
//! The whole photo stays visible and the area outside the cut-out is dimmed,
//! which is the standard way to frame a crop. The view is fitted to the *union*
//! of the photo and the crop window rather than to the photo alone, so a window
//! dragged partly or wholly off the picture stays on screen instead of
//! disappearing past the edge — that window is a legitimate framing choice
//! here, not a mistake, and it previews the printed border.

use std::hash::{Hash, Hasher};

use sort4print_core::cropbox::Handle;
use sort4print_core::stamp::{self, StampStyle};

use crate::app::{DragState, Sort4Print};
use crate::ui::ACCENT;

/// Grab radius for the handles, in screen pixels.
const GRAB_PX: f32 = 11.0;
const HANDLE_PX: f32 = 9.0;

pub fn show(app: &mut Sort4Print, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        if app.entries.is_empty() {
            empty_state(ui);
            return;
        }
        controls(app, ui);
        ui.separator();
        canvas(app, ui);
    });
}

fn empty_state(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new(
                "Open a folder of photos to begin.\n\n\
                 Space picks the current photo · ← → walk the folder · \
                 drag to move the crop, drag a handle to resize it, scroll to zoom.",
            )
            .weak(),
        );
    });
}

fn controls(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("Previous (←)").clicked() {
            app.step(-1);
        }
        if ui.button("▶").on_hover_text("Next (→)").clicked() {
            app.step(1);
        }

        let index = app.current;
        let mut selected = app.entries[index].selected;
        if ui
            .checkbox(&mut selected, "Print this one")
            .on_hover_text("Space")
            .changed()
        {
            app.entries[index].selected = selected;
        }

        ui.separator();
        if ui
            .button("Reset crop")
            .on_hover_text("Back to the largest centred window that fits")
            .clicked()
        {
            app.reset_crop(index);
        }

        ui.separator();
        ui.label(
            egui::RichText::new("drag to move · handles resize · scroll zooms")
                .small()
                .weak(),
        );
    });
}

fn canvas(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let index = app.current;
    let path = app.entries[index].path.clone();

    let Some(image) = app.prefetch.preview(&path) else {
        let message = match app.prefetch.error(&path) {
            Some(error) => format!("Could not read this file:\n{error}"),
            None => "Loading…".to_string(),
        };
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new(message).weak());
        });
        return;
    };

    let texture = app.texture_for(ui.ctx(), &path, &image);
    let full_w = image.preview.full_w as f64;
    let full_h = image.preview.full_h as f64;
    let mut crop = app.crop_for(index, image.preview.full_w, image.preview.full_h);
    let ratio = app.ratio_for(image.preview.full_w, image.preview.full_h);

    let area = ui.available_rect_before_wrap();
    let response = ui.interact(
        area,
        ui.id().with("crop-canvas"),
        egui::Sense::click_and_drag(),
    );

    // Fit the photo and the crop window together, with a little air around them.
    let world_min_x = crop.x.min(0.0);
    let world_min_y = crop.y.min(0.0);
    let world_max_x = crop.right().max(full_w);
    let world_max_y = crop.bottom().max(full_h);
    let world_w = (world_max_x - world_min_x).max(1.0);
    let world_h = (world_max_y - world_min_y).max(1.0);

    let scale = ((area.width() as f64 / world_w).min(area.height() as f64 / world_h) * 0.94)
        .max(f64::MIN_POSITIVE);
    let drawn = egui::vec2((world_w * scale) as f32, (world_h * scale) as f32);
    let origin = area.center() - drawn / 2.0;

    let to_screen = |x: f64, y: f64| {
        egui::pos2(
            origin.x + ((x - world_min_x) * scale) as f32,
            origin.y + ((y - world_min_y) * scale) as f32,
        )
    };
    let to_world = |p: egui::Pos2| {
        (
            (p.x - origin.x) as f64 / scale + world_min_x,
            (p.y - origin.y) as f64 / scale + world_min_y,
        )
    };

    // ---- interaction ----------------------------------------------------

    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let world = to_world(pos);
            let tolerance = GRAB_PX as f64 / scale;
            if let Some(handle) = crop.hit_test(world.0, world.1, tolerance) {
                app.drag = Some(DragState {
                    handle,
                    last: world,
                });
            }
        }
    }

    if response.dragged() {
        if let (Some(drag), Some(pos)) = (app.drag.as_ref(), response.interact_pointer_pos()) {
            let world = to_world(pos);
            crop = match drag.handle {
                Handle::Move => crop.translated(world.0 - drag.last.0, world.1 - drag.last.1),
                handle => crop.dragged(handle, world, ratio),
            };
            if let Some(drag) = app.drag.as_mut() {
                drag.last = world;
            }
            app.set_crop(index, crop);
        }
    }

    if response.drag_stopped() {
        app.drag = None;
    }

    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            // Scrolling up zooms in, i.e. makes the window smaller.
            let factor = (1.0 - scroll as f64 * 0.0015).clamp(0.5, 2.0);
            crop = crop.scaled_about_center(factor);
            app.set_crop(index, crop);
        }
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            let world = to_world(pos);
            let tolerance = GRAB_PX as f64 / scale;
            let cursor = match crop.hit_test(world.0, world.1, tolerance) {
                Some(Handle::Move) => egui::CursorIcon::Grab,
                Some(Handle::TopLeft) | Some(Handle::BottomRight) => {
                    egui::CursorIcon::ResizeNwSe
                }
                Some(Handle::TopRight) | Some(Handle::BottomLeft) => {
                    egui::CursorIcon::ResizeNeSw
                }
                Some(Handle::Left) | Some(Handle::Right) => egui::CursorIcon::ResizeHorizontal,
                Some(Handle::Top) | Some(Handle::Bottom) => egui::CursorIcon::ResizeVertical,
                None => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
    }

    // ---- painting -------------------------------------------------------

    let painter = ui.painter_at(area);
    let image_rect = egui::Rect::from_min_max(to_screen(0.0, 0.0), to_screen(full_w, full_h));
    let crop_rect = egui::Rect::from_min_max(
        to_screen(crop.x, crop.y),
        to_screen(crop.right(), crop.bottom()),
    );

    // The window can extend past the photo; that area prints as background.
    let background = app.config.background;
    painter.rect_filled(
        crop_rect,
        0.0,
        egui::Color32::from_rgb(background.r, background.g, background.b),
    );

    painter.image(
        texture.id(),
        image_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    dim_outside(&painter, area, crop_rect);
    draw_caption_overlay(app, ui, index, crop_rect, &image);

    painter.rect_stroke(
        crop_rect,
        0.0,
        egui::Stroke::new(1.5, ACCENT),
        egui::StrokeKind::Middle,
    );
    thirds(&painter, crop_rect);
    handles(&painter, crop_rect);

    // ---- readout --------------------------------------------------------

    let (_, _, out_w, out_h) = crop.to_pixel_rect();
    let mut lines = vec![format!("{out_w} × {out_h} px")];
    if crop.overflows(full_w, full_h) {
        lines.push("window extends past the photo — the rest prints as background".into());
    }
    painter.text(
        egui::pos2(area.left() + 8.0, area.top() + 6.0),
        egui::Align2::LEFT_TOP,
        lines.join("   ·   "),
        egui::FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
}

fn dim_outside(painter: &egui::Painter, area: egui::Rect, crop: egui::Rect) {
    let shade = egui::Color32::from_black_alpha(150);
    let top = egui::Rect::from_min_max(area.min, egui::pos2(area.right(), crop.top()));
    let bottom = egui::Rect::from_min_max(egui::pos2(area.left(), crop.bottom()), area.max);
    let left = egui::Rect::from_min_max(
        egui::pos2(area.left(), crop.top()),
        egui::pos2(crop.left(), crop.bottom()),
    );
    let right = egui::Rect::from_min_max(
        egui::pos2(crop.right(), crop.top()),
        egui::pos2(area.right(), crop.bottom()),
    );
    for rect in [top, bottom, left, right] {
        if rect.is_positive() {
            painter.rect_filled(rect, 0.0, shade);
        }
    }
}

fn thirds(painter: &egui::Painter, rect: egui::Rect) {
    let stroke = egui::Stroke::new(0.7, egui::Color32::from_white_alpha(60));
    for i in 1..3 {
        let t = i as f32 / 3.0;
        let x = rect.left() + rect.width() * t;
        let y = rect.top() + rect.height() * t;
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], stroke);
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], stroke);
    }
}

fn handles(painter: &egui::Painter, rect: egui::Rect) {
    let points = [
        rect.left_top(),
        rect.center_top(),
        rect.right_top(),
        rect.right_center(),
        rect.right_bottom(),
        rect.center_bottom(),
        rect.left_bottom(),
        rect.left_center(),
    ];
    for point in points {
        let handle = egui::Rect::from_center_size(point, egui::vec2(HANDLE_PX, HANDLE_PX));
        painter.rect_filled(handle, 1.0, ACCENT);
        painter.rect_stroke(
            handle,
            1.0,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
            egui::StrokeKind::Middle,
        );
    }
}

/// Renders the caption to a transparent overlay the size of the crop rectangle
/// on screen, so what is shown is the export's own renderer at preview scale
/// rather than an approximation drawn with egui's text stack.
fn draw_caption_overlay(
    app: &mut Sort4Print,
    ui: &mut egui::Ui,
    index: usize,
    crop_rect: egui::Rect,
    image: &crate::prefetch::LoadedImage,
) {
    if !app.config.caption.enabled {
        return;
    }
    let text = app.caption_for(index, Some(image));
    if text.trim().is_empty() {
        return;
    }
    let Some(font) = app.caption_font() else {
        return;
    };

    // Rounded so that dragging the crop by a pixel does not re-render.
    let w = (crop_rect.width().max(16.0) / 8.0).round() as u32 * 8;
    let h = (crop_rect.height().max(16.0) / 8.0).round() as u32 * 8;
    if w == 0 || h == 0 {
        return;
    }

    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        w.hash(&mut hasher);
        h.hash(&mut hasher);
        app.config.caption.font_family.hash(&mut hasher);
        app.config.caption.font_style.hash(&mut hasher);
        app.config.caption.corner.as_str().hash(&mut hasher);
        (app.config.caption.size_pct.to_bits()).hash(&mut hasher);
        (app.config.caption.outline_pct.to_bits()).hash(&mut hasher);
        (app.config.caption.margin_pct.to_bits()).hash(&mut hasher);
        app.config.caption.fill.to_hex().hash(&mut hasher);
        app.config.caption.outline.to_hex().hash(&mut hasher);
        hasher.finish()
    };

    let cached = app
        .caption_overlay
        .as_ref()
        .filter(|(cached_key, _)| *cached_key == key)
        .map(|(_, texture)| texture.clone());

    let texture = match cached {
        Some(texture) => texture,
        None => {
            let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 0]));
            let style = StampStyle::from_config(&app.config.caption, w, h);
            stamp::draw_caption(&mut canvas, &text, &font, &style);
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                canvas.as_raw(),
            );
            let texture =
                ui.ctx()
                    .load_texture("caption-overlay", color, egui::TextureOptions::LINEAR);
            app.caption_overlay = Some((key, texture.clone()));
            texture
        }
    };

    ui.painter_at(crop_rect).image(
        texture.id(),
        crop_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// Shown in the settings panel: the caption rendered by the real renderer on a
/// ramp that runs light to dark, so both the fill and the outline can be judged.
pub fn caption_swatch(app: &mut Sort4Print, ui: &mut egui::Ui, sample_text: &str) {
    let (w, h) = (420u32, 130u32);
    let Some(font) = app.caption_font() else {
        ui.weak("No font could be loaded.");
        return;
    };

    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        sample_text.hash(&mut hasher);
        app.config.caption.font_family.hash(&mut hasher);
        app.config.caption.font_style.hash(&mut hasher);
        app.config.caption.corner.as_str().hash(&mut hasher);
        app.config.caption.size_pct.to_bits().hash(&mut hasher);
        app.config.caption.outline_pct.to_bits().hash(&mut hasher);
        app.config.caption.margin_pct.to_bits().hash(&mut hasher);
        app.config.caption.fill.to_hex().hash(&mut hasher);
        app.config.caption.outline.to_hex().hash(&mut hasher);
        app.config.caption.uppercase.hash(&mut hasher);
        hasher.finish()
    };

    let cached = app
        .caption_swatch
        .as_ref()
        .filter(|(cached_key, _)| *cached_key == key)
        .map(|(_, texture)| texture.clone());

    let texture = match cached {
        Some(texture) => texture,
        None => {
            let mut canvas = stamp::preview_backdrop(w, h);
            // The swatch is far smaller than a real print, so scale the style
            // up to keep the caption legible while preserving its proportions.
            let style = StampStyle::from_config(&app.config.caption, w * 4, h * 4);
            let style = StampStyle {
                size_px: style.size_px / 4.0,
                outline_px: style.outline_px / 4.0,
                margin_px: style.margin_px / 4.0,
                ..style
            };
            stamp::draw_caption(&mut canvas, sample_text, &font, &style);
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                canvas.as_raw(),
            );
            let texture =
                ui.ctx()
                    .load_texture("caption-swatch", color, egui::TextureOptions::LINEAR);
            app.caption_swatch = Some((key, texture.clone()));
            texture
        }
    };

    ui.add(egui::Image::new(&texture).corner_radius(3.0));
}
