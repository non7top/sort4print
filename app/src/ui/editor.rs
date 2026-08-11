//! The crop editor.
//!
//! The whole photo stays visible and the area outside the cut-out is dimmed,
//! which is the standard way to frame a crop. The view is fitted to the *union*
//! of the photo and the crop window rather than to the photo alone, so a window
//! reaching past the edge stays on screen instead of disappearing — that window
//! is a legitimate framing choice here, and it previews the printed border.
//!
//! Two zooms live here and are deliberately kept apart. The wheel resizes the
//! *crop window*, which is the thing you are usually adjusting. Ctrl and the
//! wheel magnifies the *view*, for looking closely at what you are about to
//! print; it changes nothing that gets exported.

use std::hash::{Hash, Hasher};

use sort4print_core::cropbox::{Constraints, CropBox, Handle};
use sort4print_core::stamp::{self, StampStyle};

use crate::app::{DragState, Sort4Print};
use crate::ui::{ACCENT, OK_GREEN};

/// Grab radius for the handles, in screen pixels.
const GRAB_PX: f32 = 11.0;
const HANDLE_PX: f32 = 9.0;

/// How far the view may be magnified beyond fitting the window.
const MAX_VIEW_ZOOM: f32 = 12.0;

/// Short side of the caption overlay raster.
///
/// Fixed on purpose. Rendering the caption at the crop's on-screen size meant
/// re-running the whole glyph raster and its distance transform every few
/// pixels of a drag, on the UI thread — which is exactly what made dragging
/// feel like it was fighting back. At a fixed size the raster is produced once
/// per caption and simply stretched, which no one can tell apart in a preview.
const CAPTION_RASTER_SHORT_SIDE: f32 = 460.0;

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
                 Space picks the current photo · ← → walk the folder\n\
                 drag to move the crop, drag a handle to resize it\n\
                 scroll zooms the crop · Ctrl+scroll zooms the view · Alt+arrows nudge",
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
            app.note_changed(index);
        }

        ui.separator();
        if ui
            .button("Reset crop")
            .on_hover_text("Back to the largest centred window that fits")
            .clicked()
        {
            app.reset_crop(index);
        }

        if app.view_zoom > 1.001 {
            if ui
                .button(format!("Fit ({:.0}%)", app.view_zoom * 100.0))
                .on_hover_text("Back to showing the whole photo")
                .clicked()
            {
                app.reset_view();
            }
        }

        ui.separator();
        ui.label(
            egui::RichText::new(
                "drag moves · handles resize · scroll zooms the crop · Ctrl+scroll the view",
            )
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
    let (image_w, image_h) = (image.preview.full_w, image.preview.full_h);
    let full_w = image_w as f64;
    let full_h = image_h as f64;
    let mut crop = app.crop_for(index, image_w, image_h);
    let ratio = app.ratio_for(image_w, image_h);

    let area = ui.available_rect_before_wrap();

    // Record how big this really is in screen pixels, which is what decides how
    // sharp a preview needs to be. A laptop panel and an external monitor differ
    // enough for it to be worth measuring rather than assuming.
    let scale_factor = ui.ctx().pixels_per_point();
    app.editor_long_px = (area.width().max(area.height()) * scale_factor).round() as u32;

    let response = ui.interact(
        area,
        ui.id().with("crop-canvas"),
        egui::Sense::click_and_drag(),
    );

    // ---- view transform --------------------------------------------------

    // The world is the photo and the crop window together, so neither can be
    // pushed off screen by the other.
    let world_min_x = crop.x.min(0.0);
    let world_min_y = crop.y.min(0.0);
    let world_w = (crop.right().max(full_w) - world_min_x).max(1.0);
    let world_h = (crop.bottom().max(full_h) - world_min_y).max(1.0);

    let fit = ((area.width() as f64 / world_w).min(area.height() as f64 / world_h) * 0.94)
        .max(f64::MIN_POSITIVE);
    let scale = fit * app.view_zoom as f64;
    let drawn = egui::vec2((world_w * scale) as f32, (world_h * scale) as f32);
    let origin = area.center() - drawn / 2.0 + app.view_pan;

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

    let constraints = app.constraints_for(image_w, image_h, scale);

    // ---- interaction -----------------------------------------------------

    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let world = to_world(pos);
            let tolerance = GRAB_PX as f64 / scale;
            if let Some(handle) = crop.hit_test(world.0, world.1, tolerance) {
                app.drag = Some(DragState {
                    handle,
                    start_pointer: world,
                    start_box: crop,
                });
            }
        }
    }

    if response.dragged() {
        // Dragging with the middle button, or anywhere off the crop window,
        // slides the view rather than the crop.
        let panning = app.drag.is_none()
            || ui.input(|i| i.pointer.middle_down());
        if panning {
            app.view_pan += response.drag_delta();
        } else if let (Some(drag), Some(pos)) = (app.drag.as_ref(), response.interact_pointer_pos())
        {
            let world = to_world(pos);
            crop = match drag.handle {
                // Always measured from where the drag began, so the snap that
                // is applied afterwards never eats the movement that would have
                // escaped it.
                Handle::Move => drag.start_box.apply_move(
                    world.0 - drag.start_pointer.0,
                    world.1 - drag.start_pointer.1,
                    constraints,
                ),
                // Resizing is already absolute: it works from the pointer and
                // the anchored edge, neither of which is affected by the last
                // frame's snapping.
                handle => drag.start_box.apply_drag(handle, world, ratio, constraints),
            };
            app.set_crop(index, crop);
        }
    }

    if response.drag_stopped() {
        app.drag = None;
    }

    if response.hovered() {
        // egui does not hand ctrl+scroll through as a scroll: it recognises the
        // gesture and re-emits it as a zoom, leaving the scroll delta at zero.
        // So the view zoom has to be read from `zoom_delta`, and only the
        // leftover plain scrolling belongs to the crop.
        let (scroll, zoom_delta, ctrl) = ui.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.zoom_delta(),
                i.modifiers.command || i.modifiers.ctrl,
            )
        });

        let view_factor = if (zoom_delta - 1.0).abs() > 0.001 {
            Some(zoom_delta)
        } else if ctrl && scroll.abs() > 0.1 {
            // Belt and braces: if a platform ever delivers it as a plain
            // scroll with the modifier held, treat it the same way.
            Some(1.0 + scroll * 0.0015)
        } else {
            None
        };

        if let Some(factor) = view_factor {
            view_zoom(app, ui, area, factor, world_min_x, world_min_y, world_w, world_h, fit);
        } else if scroll.abs() > 0.1 {
            // Scrolling up zooms in, i.e. makes the window smaller.
            let factor = (1.0 - scroll as f64 * 0.0015).clamp(0.5, 2.0);
            crop = crop.apply_zoom(factor, ratio, constraints);
            app.set_crop(index, crop);
        }

        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            let world = to_world(pos);
            let tolerance = GRAB_PX as f64 / scale;
            let cursor = match crop.hit_test(world.0, world.1, tolerance) {
                Some(Handle::Move) => egui::CursorIcon::Grab,
                Some(Handle::TopLeft) | Some(Handle::BottomRight) => egui::CursorIcon::ResizeNwSe,
                Some(Handle::TopRight) | Some(Handle::BottomLeft) => egui::CursorIcon::ResizeNeSw,
                Some(Handle::Left) | Some(Handle::Right) => egui::CursorIcon::ResizeHorizontal,
                Some(Handle::Top) | Some(Handle::Bottom) => egui::CursorIcon::ResizeVertical,
                None => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
    }

    // ---- painting --------------------------------------------------------

    let painter = ui.painter_at(area);
    let image_rect = egui::Rect::from_min_max(to_screen(0.0, 0.0), to_screen(full_w, full_h));
    let crop_rect = egui::Rect::from_min_max(
        to_screen(crop.x, crop.y),
        to_screen(crop.right(), crop.bottom()),
    );

    // Everything around the picture answers "is this one going to be printed?"
    // without having to look anywhere else on screen.
    let picked = app.current_is_selected();
    painter.rect_filled(area, 0.0, surround_colour(ui, picked));

    // The window may extend past the photo; that area prints as background.
    // This one is *not* tinted by the pick state — it is a preview of the actual
    // printed border, and showing it as anything other than its real colour
    // would be a lie about the output.
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

    dim_outside(&painter, area, crop_rect, picked);
    draw_caption_overlay(app, ui, index, crop_rect, &image);

    // Picked photos get a green window, so the decision is visible without
    // looking away from the picture.
    let outline = if picked { OK_GREEN } else { ACCENT };

    painter.rect_stroke(
        crop_rect,
        0.0,
        egui::Stroke::new(1.5, outline),
        egui::StrokeKind::Middle,
    );
    thirds(&painter, crop_rect);
    handles(&painter, crop_rect, outline);

    // ---- readout ---------------------------------------------------------

    let (_, _, out_w, out_h) = crop.to_pixel_rect();
    let mut lines = vec![format!("{out_w} × {out_h} px")];
    if crop.overflows(full_w, full_h) {
        lines.push("window past the photo — the rest prints as background".into());
    }
    if app.view_zoom > 1.001 {
        lines.push(format!("view {:.0}%", app.view_zoom * 100.0));
    }
    painter.text(
        egui::pos2(area.left() + 8.0, area.top() + 6.0),
        egui::Align2::LEFT_TOP,
        lines.join("   ·   "),
        egui::FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );

    if picked {
        picked_badge(&painter, area);
    }
}

/// Says it in words as well as in colour, for the same reason a traffic light
/// has positions: colour alone is not something everyone can read.
fn picked_badge(painter: &egui::Painter, area: egui::Rect) {
    let galley = painter.layout_no_wrap(
        "✔ WILL BE PRINTED".to_owned(),
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
    );
    let padding = egui::vec2(10.0, 5.0);
    let size = galley.size() + padding * 2.0;
    let rect = egui::Rect::from_min_size(
        egui::pos2(area.right() - size.x - 10.0, area.top() + 6.0),
        size,
    );
    painter.rect_filled(rect, 4.0, OK_GREEN);
    painter.galley(rect.min + padding, galley, egui::Color32::WHITE);
}

/// Magnifies the view about the pointer, so whatever is under it stays there.
/// `factor` is multiplicative: above 1 moves closer.
#[allow(clippy::too_many_arguments)]
fn view_zoom(
    app: &mut Sort4Print,
    ui: &egui::Ui,
    area: egui::Rect,
    factor: f32,
    world_min_x: f64,
    world_min_y: f64,
    world_w: f64,
    world_h: f64,
    fit: f64,
) {
    let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) else {
        return;
    };

    let old_zoom = app.view_zoom;
    let new_zoom = (old_zoom * factor).clamp(1.0, MAX_VIEW_ZOOM);
    if (new_zoom - old_zoom).abs() < f32::EPSILON {
        return;
    }

    // Where the pointer is, in world coordinates, before the change.
    let old_scale = fit * old_zoom as f64;
    let old_drawn = egui::vec2((world_w * old_scale) as f32, (world_h * old_scale) as f32);
    let old_origin = area.center() - old_drawn / 2.0 + app.view_pan;
    let world_x = (pointer.x - old_origin.x) as f64 / old_scale + world_min_x;
    let world_y = (pointer.y - old_origin.y) as f64 / old_scale + world_min_y;

    // Choose the pan that puts that same world point back under the pointer.
    let new_scale = fit * new_zoom as f64;
    let new_drawn = egui::vec2((world_w * new_scale) as f32, (world_h * new_scale) as f32);
    let wanted_origin = egui::pos2(
        pointer.x - ((world_x - world_min_x) * new_scale) as f32,
        pointer.y - ((world_y - world_min_y) * new_scale) as f32,
    );

    app.view_zoom = new_zoom;
    app.view_pan = wanted_origin - (area.center() - new_drawn / 2.0);
    if new_zoom <= 1.0 {
        app.view_pan = egui::Vec2::ZERO;
    }
}

/// What fills the space around the photo. Green says "this one is going to be
/// printed"; anything else is the ordinary dark surround.
fn surround_colour(ui: &egui::Ui, picked: bool) -> egui::Color32 {
    if picked {
        egui::Color32::from_rgb(20, 46, 28)
    } else {
        ui.visuals().extreme_bg_color
    }
}

fn dim_outside(painter: &egui::Painter, area: egui::Rect, crop: egui::Rect, picked: bool) {
    // Tinting the shade rather than only the empty space means the whole of the
    // photo outside the crop carries the signal too, which is a much larger
    // target for the eye than a border would be.
    let shade = if picked {
        egui::Color32::from_rgba_unmultiplied(18, 92, 44, 150)
    } else {
        egui::Color32::from_black_alpha(150)
    };
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

fn handles(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
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
        painter.rect_filled(handle, 1.0, color);
        painter.rect_stroke(
            handle,
            1.0,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
            egui::StrokeKind::Middle,
        );
    }
}

/// Renders the caption to a transparent overlay and stretches it over the crop,
/// so what is shown comes from the export's own renderer rather than being
/// approximated with egui's text stack.
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

    // The raster's proportions match the crop so the percentage-based sizes
    // land in the same relative place; its scale is fixed so dragging never
    // triggers a re-render.
    let aspect = (crop_rect.width() / crop_rect.height().max(1.0)).clamp(0.05, 20.0);
    let (w, h) = if aspect >= 1.0 {
        ((CAPTION_RASTER_SHORT_SIDE * aspect) as u32, CAPTION_RASTER_SHORT_SIDE as u32)
    } else {
        (CAPTION_RASTER_SHORT_SIDE as u32, (CAPTION_RASTER_SHORT_SIDE / aspect) as u32)
    };
    if w == 0 || h == 0 {
        return;
    }

    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        w.hash(&mut hasher);
        h.hash(&mut hasher);
        caption_style_key(app, &mut hasher);
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
            let color =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], canvas.as_raw());
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

fn caption_style_key(app: &Sort4Print, hasher: &mut impl Hasher) {
    let c = &app.config.caption;
    c.font_family.hash(hasher);
    c.font_style.hash(hasher);
    c.corner.as_str().hash(hasher);
    c.size_pct.to_bits().hash(hasher);
    c.outline_pct.to_bits().hash(hasher);
    c.margin_pct.to_bits().hash(hasher);
    c.fill.to_hex().hash(hasher);
    c.outline.to_hex().hash(hasher);
    c.uppercase.hash(hasher);
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
        caption_style_key(app, &mut hasher);
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
            let color =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], canvas.as_raw());
            let texture =
                ui.ctx()
                    .load_texture("caption-swatch", color, egui::TextureOptions::LINEAR);
            app.caption_swatch = Some((key, texture.clone()));
            texture
        }
    };

    ui.add(egui::Image::new(&texture).corner_radius(3.0));
}

/// Keeps the unused-import warning honest: `Constraints` and `CropBox` are part
/// of this module's vocabulary even though the compiler only sees them through
/// inference above.
#[allow(dead_code)]
fn _type_anchors(c: Constraints, b: CropBox) -> (Constraints, CropBox) {
    (c, b)
}
