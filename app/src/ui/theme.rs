//! The look of the thing.
//!
//! Windows 10's control style, which is worth naming precisely because it is
//! not Windows 11's: square corners rather than rounded, a hairline border
//! around everything that can be clicked, a shallow top-to-bottom gradient in
//! the fill, and one saturated accent. Controls read as raised objects sitting
//! on a surface, and the surface itself is a shade apart from the controls on
//! it. That is what makes elements tell themselves apart without any labelling.
//!
//! egui's stock theme paints buttons as flat rectangles the same colour as the
//! panel behind them, which is why they read as text rather than as buttons.
//! Two things fix that, and both live here: a `Style` with real borders and
//! distinct per-state fills, and a button that paints its own gradient, since a
//! theme alone cannot express one.
//!
//! Colour carries meaning as well as decoration. A control is drawn in the
//! `Tone` of what it does — opening something, writing something out, throwing
//! something away — so the destructive ones cannot be mistaken for the routine
//! ones at a glance.

use egui::{Color32, CornerRadius, Stroke};

pub use sort4print_core::config::Theme;

/// Windows' own accent blue. Recognisable enough that using anything else for
/// selection would look like a mistake.
pub const ACCENT: Color32 = Color32::from_rgb(0, 120, 215);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0, 84, 153);

/// The crop window's handles and outline. Amber against both palettes and
/// against a photograph, which grey or blue would not be.
pub const CROP_AMBER: Color32 = Color32::from_rgb(255, 176, 32);
pub const PICKED_GREEN: Color32 = Color32::from_rgb(76, 175, 80);
pub const WARN_AMBER: Color32 = Color32::from_rgb(214, 140, 20);
pub const DANGER_RED: Color32 = Color32::from_rgb(196, 62, 54);

/// The area a photograph is judged against stays dark whatever the chrome does.
/// A light surround changes how you read the exposure of the thing sitting on
/// it, which is the one job this program has.
pub const CANVAS: Color32 = Color32::from_rgb(38, 38, 40);
/// The same, for a picked photo: a hint of green, never enough to cast on the
/// picture.
pub const CANVAS_PICKED: Color32 = Color32::from_rgb(28, 46, 33);

pub struct Palette {
    pub dark: bool,
    /// Behind the panels.
    pub surface: Color32,
    /// Behind the side panels, a shade apart from `surface` so the regions of
    /// the window are visibly separate things.
    pub surface_alt: Color32,
    /// Sunken areas: text fields, wells, list backgrounds.
    pub sunken: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// Hairline around anything interactive.
    pub border: Color32,
    /// The stronger line under a raised control, which is what gives it depth.
    pub border_strong: Color32,
    /// Ground behind a picked row in the list.
    pub picked_row: Color32,
}

impl Palette {
    pub fn of(theme: Theme) -> Palette {
        match theme {
            Theme::Light => Palette {
                dark: false,
                surface: Color32::from_rgb(240, 240, 240),
                surface_alt: Color32::from_rgb(230, 232, 235),
                sunken: Color32::from_rgb(252, 252, 252),
                text: Color32::from_rgb(28, 28, 28),
                text_weak: Color32::from_rgb(104, 104, 104),
                border: Color32::from_rgb(172, 172, 172),
                border_strong: Color32::from_rgb(130, 130, 130),
                picked_row: Color32::from_rgb(214, 238, 216),
            },
            Theme::Dark => Palette {
                dark: true,
                surface: Color32::from_rgb(45, 45, 48),
                surface_alt: Color32::from_rgb(37, 37, 40),
                sunken: Color32::from_rgb(30, 30, 32),
                text: Color32::from_rgb(238, 238, 238),
                text_weak: Color32::from_rgb(160, 160, 160),
                border: Color32::from_rgb(80, 80, 84),
                border_strong: Color32::from_rgb(24, 24, 26),
                picked_row: Color32::from_rgb(32, 56, 38),
            },
        }
    }
}

/// What a control does, which decides how it is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// Ordinary. Grey, like most of Windows.
    #[default]
    Neutral,
    /// The thing this screen is for.
    Primary,
    /// Produces output.
    Positive,
    /// Takes a while, or changes a lot at once.
    Caution,
    /// Discards something.
    Danger,
}

impl Tone {
    /// Top and bottom of the gradient, the border, and the text.
    fn colours(self, palette: &Palette, hovered: bool, pressed: bool) -> ToneColours {
        let (top, bottom, border, text) = match self {
            Tone::Neutral => {
                if palette.dark {
                    (
                        Color32::from_rgb(72, 72, 76),
                        Color32::from_rgb(58, 58, 62),
                        palette.border,
                        palette.text,
                    )
                } else {
                    (
                        Color32::from_rgb(252, 252, 252),
                        Color32::from_rgb(228, 228, 228),
                        palette.border,
                        palette.text,
                    )
                }
            }
            Tone::Primary => (
                Color32::from_rgb(38, 148, 235),
                ACCENT,
                ACCENT_DARK,
                Color32::WHITE,
            ),
            Tone::Positive => (
                Color32::from_rgb(96, 190, 100),
                Color32::from_rgb(56, 142, 60),
                Color32::from_rgb(40, 106, 44),
                Color32::WHITE,
            ),
            Tone::Caution => (
                Color32::from_rgb(238, 170, 50),
                Color32::from_rgb(206, 132, 16),
                Color32::from_rgb(150, 96, 10),
                Color32::from_rgb(38, 26, 0),
            ),
            Tone::Danger => (
                Color32::from_rgb(216, 88, 80),
                Color32::from_rgb(180, 52, 46),
                Color32::from_rgb(132, 38, 34),
                Color32::WHITE,
            ),
        };

        // Hovering lifts the whole face, pressing sinks it and flips the
        // gradient — the same two cues Windows uses, and the reason a control
        // feels like it has been pushed rather than merely recoloured.
        let (top, bottom) = if pressed {
            (shift(bottom, -0.06), shift(top, -0.10))
        } else if hovered {
            (shift(top, 0.06), shift(bottom, 0.06))
        } else {
            (top, bottom)
        };

        ToneColours {
            top,
            bottom,
            border,
            text,
        }
    }
}

struct ToneColours {
    top: Color32,
    bottom: Color32,
    border: Color32,
    text: Color32,
}

/// Moves a colour towards white (positive) or black (negative).
fn shift(colour: Color32, amount: f32) -> Color32 {
    let mix = |channel: u8| -> u8 {
        let target = if amount >= 0.0 { 255.0 } else { 0.0 };
        let t = amount.abs().clamp(0.0, 1.0);
        (channel as f32 * (1.0 - t) + target * t).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(mix(colour.r()), mix(colour.g()), mix(colour.b()))
}

/// Installs the palette, the metrics and the fonts.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let palette = Palette::of(theme);

    // egui 0.36 keeps one style per theme and picks between them, rather than
    // holding a single style. So the palette goes into that theme's slot and
    // the theme is then selected, instead of overwriting one global style.
    let slot = if palette.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let mut style = (*ctx.style_of(slot)).clone();

    // Square corners: two pixels, not the eight of a modern rounded look.
    let radius = CornerRadius::same(2);
    let border = Stroke::new(1.0, palette.border);

    let mut visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.dark_mode = palette.dark;
    visuals.panel_fill = palette.surface;
    visuals.window_fill = palette.surface;
    visuals.extreme_bg_color = palette.sunken;
    visuals.faint_bg_color = if palette.dark {
        Color32::from_rgb(52, 52, 56)
    } else {
        Color32::from_rgb(246, 246, 246)
    };
    visuals.text_edit_bg_color = Some(palette.sunken);
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.text_weak);
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.selection = egui::style::Selection {
        bg_fill: ACCENT,
        stroke: Stroke::new(1.0, Color32::WHITE),
    };
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARN_AMBER;
    visuals.error_fg_color = DANGER_RED;
    visuals.striped = true;
    visuals.slider_trailing_fill = true;
    // Buttons that egui draws itself still get a frame; the custom ones below
    // paint their own.
    visuals.button_frame = true;

    // Every state visibly different from the others, which is what egui's stock
    // theme does not do.
    let neutral_face = if palette.dark {
        Color32::from_rgb(64, 64, 68)
    } else {
        Color32::from_rgb(240, 240, 240)
    };
    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.weak_bg_fill = palette.surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.noninteractive.corner_radius = radius;

    visuals.widgets.inactive.bg_fill = neutral_face;
    visuals.widgets.inactive.weak_bg_fill = neutral_face;
    visuals.widgets.inactive.bg_stroke = border;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.bg_fill = shift(neutral_face, if palette.dark { 0.10 } else { 0.02 });
    visuals.widgets.hovered.weak_bg_fill =
        shift(neutral_face, if palette.dark { 0.10 } else { 0.02 });
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = shift(ACCENT, -0.10);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_DARK);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = neutral_face;
    visuals.widgets.open.weak_bg_fill = neutral_face;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.open.corner_radius = radius;

    style.visuals = visuals;

    // Desktop metrics: controls tall enough to hit, and room around them so the
    // borders have something to sit in.
    style.spacing.item_spacing = egui::vec2(7.0, 6.0);
    style.spacing.button_padding = egui::vec2(11.0, 5.0);
    style.spacing.interact_size = egui::vec2(44.0, 26.0);
    style.spacing.slider_width = 150.0;
    style.spacing.icon_width = 17.0;
    style.spacing.icon_width_inner = 9.0;
    style.spacing.combo_width = 110.0;

    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(17.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(13.5, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Small, FontId::new(11.5, FontFamily::Proportional));

    ctx.set_style_of(slot, style);
    ctx.set_theme(slot);
}

/// Uses the system's own interface font when it can be found.
///
/// Segoe UI is what every other window on the machine is set in, and matching
/// it does more for looking native than any amount of border work. Failing to
/// find it is not a problem: egui's built-in font is perfectly legible, just
/// not local.
pub fn apply_system_font(ctx: &egui::Context, catalog: &sort4print_core::fonts::FontCatalog) {
    const CANDIDATES: &[(&str, &str)] = &[
        ("Segoe UI", "Regular"),
        ("Segoe UI", "Semilight"),
        ("Selawik", "Regular"),
        ("DejaVu Sans", "Book"),
    ];

    for (family, style) in CANDIDATES {
        let Some(face) = catalog.find(family, style) else {
            continue;
        };
        let Ok(bytes) = face.read() else { continue };

        let mut fonts = egui::FontDefinitions::default();
        let name = format!("{family} {style}");
        fonts
            .font_data
            .insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, name.clone());
        ctx.set_fonts(fonts);
        crate::diagnostics::log(&format!("interface font: {name}"));
        return;
    }
    crate::diagnostics::log("interface font: egui's built-in (no system face matched)");
}

/// Paints a top-to-bottom gradient.
///
/// egui has no gradient of its own, so this is two triangles with coloured
/// corners. It is the whole reason a control here looks like an object rather
/// than a rectangle.
pub fn vertical_gradient(painter: &egui::Painter, rect: egui::Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// A button that looks like one: gradient face, hairline border, a lighter top
/// edge and a darker bottom edge.
pub fn button(ui: &mut egui::Ui, text: &str, tone: Tone) -> egui::Response {
    button_sized(ui, text, tone, None)
}

/// The same, at a chosen width — for rows of buttons that should line up.
pub fn button_sized(
    ui: &mut egui::Ui,
    text: &str,
    tone: Tone,
    width: Option<f32>,
) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        Color32::WHITE,
    );
    let height = (galley.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y);
    let size = egui::vec2(
        width.unwrap_or(galley.size().x + padding.x * 2.0),
        height,
    );

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let theme = current_theme(ui);
    let palette = Palette::of(theme);
    let colours = tone.colours(
        &palette,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );

    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(2);

    vertical_gradient(&painter, rect.shrink(1.0), colours.top, colours.bottom);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, colours.border),
        egui::StrokeKind::Inside,
    );
    // The bevel: one bright line inside the top edge.
    painter.line_segment(
        [
            rect.left_top() + egui::vec2(1.5, 1.5),
            rect.right_top() + egui::vec2(-1.5, 1.5),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(if palette.dark { 26 } else { 150 })),
    );

    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        colours.text,
    );

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// One segment of a segmented control, as Windows draws a set of mutually
/// exclusive choices: joined together, the chosen one filled with the accent.
pub fn segment(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let tone = if selected { Tone::Primary } else { Tone::Neutral };
    let response = button(ui, text, tone);
    if selected {
        let painter = ui.painter_at(response.rect);
        painter.rect_stroke(
            response.rect,
            CornerRadius::same(2),
            Stroke::new(1.0, ACCENT_DARK),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// A titled group with a coloured edge, so a panel of settings reads as
/// sections rather than one long list.
pub fn group<R>(
    ui: &mut egui::Ui,
    title: &str,
    accent: Color32,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let palette = Palette::of(current_theme(ui));
    ui.add_space(6.0);

    let response = egui::Frame::new()
        .fill(if palette.dark {
            Color32::from_rgb(52, 52, 56)
        } else {
            Color32::from_rgb(248, 248, 248)
        })
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title.to_uppercase())
                    .small()
                    .strong()
                    .color(accent),
            );
            ui.add_space(3.0);
            contents(ui)
        });

    // The coloured spine down the left, which is what makes the sections
    // countable at a glance.
    let spine = egui::Rect::from_min_size(
        response.response.rect.min,
        egui::vec2(3.0, response.response.rect.height()),
    );
    ui.painter().rect_filled(spine, CornerRadius::same(2), accent);

    response.inner
}

/// Which palette is in force, worked out from the style rather than passed
/// around, so a widget can be dropped anywhere without threading it through.
fn current_theme(ui: &egui::Ui) -> Theme {
    if ui.visuals().dark_mode {
        Theme::Dark
    } else {
        Theme::Light
    }
}
