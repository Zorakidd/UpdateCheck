//! The window's look: its colors in light and dark mode, its fonts, and the few widgets it draws itself so they
//! look like Windows 11 rather than like egui's defaults.

use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use eframe::egui;
use egui::epaint::PathStroke;
use egui::{
    Align2, Color32, Context, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Frame, Id,
    Margin, Rect, Response, RichText, Role, Sense, Shadow, Stroke, StrokeKind, Style, TextStyle, Theme, Ui,
    Vec2, WidgetInfo, pos2, vec2,
};

use crate::system;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub card_done: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    /// Darker than `success` in light mode, so small text keeps enough contrast on white.
    pub success_text: Color32,
    pub subtle: Color32,
    pub subtle_hover: Color32,
    pub selected: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    pub danger_soft: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Palette = Palette {
    bg: hex(0xF3F4F6),
    card: hex(0xFFFFFF),
    card_done: hex(0xF9FAFB),
    border: hex(0xE5E7EB),
    text: hex(0x111827),
    muted: hex(0x646B78),
    accent: hex(0x2563EB),
    accent_hover: hex(0x1D4ED8),
    accent_soft: hex(0xBFDBFE),
    success: hex(0x16A34A),
    success_text: hex(0x15803D),
    subtle: hex(0xE5E7EB),
    subtle_hover: hex(0xD1D5DB),
    selected: hex(0xFFFFFF),
    danger: hex(0xDC2626),
    danger_hover: hex(0xB91C1C),
    danger_soft: hex(0xFEF2F2),
};

pub const DARK: Palette = Palette {
    bg: hex(0x111318),
    card: hex(0x1B1E24),
    card_done: hex(0x16181D),
    border: hex(0x2A2E36),
    text: hex(0xF3F4F6),
    muted: hex(0x9CA3AF),
    accent: hex(0x3B82F6),
    accent_hover: hex(0x2563EB),
    accent_soft: hex(0x1E3A8A),
    success: hex(0x22C55E),
    success_text: hex(0x22C55E),
    subtle: hex(0x262A31),
    subtle_hover: hex(0x323741),
    selected: hex(0x3A3F48),
    danger: hex(0xEF4444),
    danger_hover: hex(0xDC2626),
    danger_soft: hex(0x2A1416),
};

pub fn palette(ctx: &Context) -> &'static Palette {
    match ctx.theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

const BOLD: &str = "bold";

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// Segoe UI, the font of Windows itself, with egui's own fonts behind it for anything it lacks. Read from the
/// Windows folder rather than built in: it is on every PC this app runs on, and that keeps the .exe small.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let folder = system::windows_dir().join("Fonts");
    let mut load = |key: &str, files: &[&str]| {
        let data = files.iter().find_map(|file| std::fs::read(folder.join(file)).ok())?;
        fonts.font_data.insert(key.to_owned(), Arc::new(FontData::from_owned(data)));
        Some(key.to_owned())
    };
    let regular = load("segoe-ui", &["segoeui.ttf"]);
    // Semibold, as Windows 11 sets its titles, or bold where a PC has only that.
    let strong = load("segoe-ui-semibold", &["seguisb.ttf", "segoeuib.ttf"]);
    let mut proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(regular) = &regular {
        proportional.insert(0, regular.clone());
    }
    let mut heavy = proportional.clone();
    if let Some(strong) = strong {
        heavy.insert(0, strong);
    }
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name(BOLD.into()), heavy);
    fonts
}

/// Fonts and both themes' styles. The window then follows whichever theme is chosen.
pub fn install(ctx: &Context) {
    ctx.set_fonts(fonts());
    ctx.style_mut_of(Theme::Light, |style| apply(style, &LIGHT, false));
    ctx.style_mut_of(Theme::Dark, |style| apply(style, &DARK, true));
}

fn apply(style: &mut Style, p: &Palette, dark: bool) {
    style.visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = 0.4;
    style.text_styles = [
        (TextStyle::Small, regular(11.0)),
        (TextStyle::Body, regular(13.0)),
        (TextStyle::Button, regular(13.0)),
        (TextStyle::Heading, bold(20.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();
    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 6.0);
    spacing.button_padding = vec2(12.0, 7.0);
    spacing.interact_size = vec2(40.0, 34.0);
    spacing.combo_height = 320.0;
    spacing.scroll = egui::style::ScrollStyle::thin();
    spacing.scroll.floating = true;

    let visuals = &mut style.visuals;
    visuals.override_text_color = Some(p.text);
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.card;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 40 }) };
    visuals.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if dark { 90 } else { 30 }) };
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.extreme_bg_color = p.bg;
    visuals.text_edit_bg_color = Some(p.bg);
    visuals.faint_bg_color = p.card_done;
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    let radius = CornerRadius::same(8);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut widgets.inactive, p.subtle),
        (&mut widgets.hovered, p.subtle_hover),
        (&mut widgets.active, p.subtle_hover),
        (&mut widgets.open, p.subtle_hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, p.border);
        state.fg_stroke = Stroke::new(1.5, p.text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    widgets.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
}

// --- Containers ---------------------------------------------------------------------------------------------

pub fn card(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
}

/// A section's title above its cards: small capitals in the muted color.
pub fn section(ui: &mut Ui, text: &str, top: f32) {
    let p = palette(ui.ctx());
    ui.add_space(top);
    ui.label(RichText::new(text.to_uppercase()).font(bold(11.0)).color(p.muted).extra_letter_spacing(0.6));
    ui.add_space(2.0);
}

pub fn text(ui: &mut Ui, text: impl Into<String>, font: FontId, color: Color32) -> Response {
    ui.add(egui::Label::new(RichText::new(text.into()).font(font).color(color)).wrap())
}

// --- Buttons ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// What the page is for, in the accent color.
    Primary,
    Secondary,
    /// Something that can't be undone, in red.
    Danger,
}

fn button_font(look: Look) -> FontId {
    match look {
        Look::Secondary => regular(13.0),
        Look::Primary | Look::Danger => bold(13.0),
    }
}

/// How big a button with this text is.
pub fn button_size(ui: &Ui, text: &str, look: Look) -> Vec2 {
    let width = ui.painter().layout_no_wrap(text.to_owned(), button_font(look), Color32::WHITE).size().x;
    vec2((width + 28.0).max(64.0), if look == Look::Secondary { 30.0 } else { 34.0 })
}

pub fn button(ui: &mut Ui, text: &str, look: Look) -> Response {
    button_with(ui, text, look, true)
}

pub fn button_with(ui: &mut Ui, text: &str, look: Look, enabled: bool) -> Response {
    let size = button_size(ui, text, look);
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    paint_button(ui, rect, &response, text, look, enabled);
    finish(response, enabled)
}

/// A button at a place of the caller's choosing, such as centered on a card's height.
pub fn button_at(ui: &mut Ui, rect: Rect, id: Id, text: &str, look: Look, enabled: bool) -> Response {
    let enabled = enabled && ui.is_enabled();
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    paint_button(ui, rect, &response, text, look, enabled);
    finish(response, enabled)
}

fn finish(response: Response, enabled: bool) -> Response {
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

fn paint_button(ui: &Ui, rect: Rect, response: &Response, text: &str, look: Look, enabled: bool) {
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, text));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = palette(ui.ctx());
    let hot = enabled && (response.hovered() || response.is_pointer_button_down_on());
    let (fill, color) = match look {
        Look::Primary => (if hot { p.accent_hover } else { p.accent }, Color32::WHITE),
        Look::Danger => (if hot { p.danger_hover } else { p.danger }, Color32::WHITE),
        Look::Secondary => (if hot { p.subtle_hover } else { p.subtle }, p.text),
    };
    let (fill, color) = if enabled { (fill, color) } else { (fill.gamma_multiply(0.5), color.gamma_multiply(0.5)) };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), fill);
    focus_ring(ui, rect, response, 8);
    let galley = painter.layout_no_wrap(text.to_owned(), button_font(look), color);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// The ring around whatever the keyboard is on, as Windows draws it.
pub fn focus_ring(ui: &Ui, rect: Rect, response: &Response, radius: u8) {
    if response.has_focus() {
        let p = palette(ui.ctx());
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(radius + 2), Stroke::new(2.0, p.text), StrokeKind::Outside);
    }
}

// --- Check boxes, switches and choices ----------------------------------------------------------------------

pub const CHECK_SIZE: f32 = 22.0;

/// The box itself, in `fill` when ticked.
pub fn paint_check(ui: &Ui, rect: Rect, checked: bool, hovered: bool, fill: Color32) {
    let p = palette(ui.ctx());
    let painter = ui.painter();
    let radius = CornerRadius::same(6);
    if checked {
        painter.rect_filled(rect, radius, fill);
        let at = |x: f32, y: f32| pos2(rect.left() + rect.width() * x, rect.top() + rect.height() * y);
        painter.line(vec![at(0.27, 0.52), at(0.43, 0.68), at(0.74, 0.34)], PathStroke::new(2.2, Color32::WHITE));
    } else {
        painter.rect_stroke(rect.shrink(1.0), radius, Stroke::new(2.0, if hovered { p.text } else { p.muted }), StrokeKind::Inside);
    }
}

/// A check box with its text beside it, the whole row clickable. Returns whether it was clicked.
pub fn check_row(ui: &mut Ui, checked: &mut bool, text: &str, note: &str) -> Response {
    let width = ui.available_width();
    let (title, note) = check_texts(ui, text, note, width);
    let height = check_texts_height(&title, note.as_ref()).max(CHECK_SIZE);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    paint_check_row(ui, rect, rect.top(), response, checked, text, title, note)
}

/// A check row with a button at its right end that does something else with the item, such as setting it
/// aside. The row is clickable up to the button. Returns the row's response, then the button's.
pub fn check_row_with_button(
    ui: &mut Ui,
    checked: &mut bool,
    text: &str,
    note: &str,
    id: Id,
    button: &str,
) -> (Response, Response) {
    let size = button_size(ui, button, Look::Secondary);
    let width = ui.available_width();
    let (title, note) = check_texts(ui, text, note, width - size.x - 16.0);
    let texts_height = check_texts_height(&title, note.as_ref());
    let (rect, _) = ui.allocate_exact_size(vec2(width, texts_height.max(CHECK_SIZE).max(size.y)), Sense::hover());
    let row = Rect::from_min_max(rect.min, pos2(rect.right() - size.x - 16.0, rect.bottom()));
    let response = ui.interact(row, id.with("row"), Sense::click());
    let top = rect.center().y - texts_height / 2.0;
    let response = paint_check_row(ui, row, top, response, checked, text, title, note);
    let place = Rect::from_min_size(pos2(rect.right() - size.x, rect.center().y - size.y / 2.0), size);
    (response, button_at(ui, place, id, button, Look::Secondary, true))
}

/// A check row's text and note, laid out beside the box in a row `width` wide.
fn check_texts(ui: &Ui, text: &str, note: &str, width: f32) -> (Arc<egui::Galley>, Option<Arc<egui::Galley>>) {
    let p = palette(ui.ctx());
    let width = (width - CHECK_SIZE - 12.0).max(80.0);
    let title = ui.painter().layout(text.to_owned(), regular(13.0), p.text, width);
    let note = (!note.is_empty()).then(|| ui.painter().layout(note.to_owned(), regular(12.0), p.muted, width));
    (title, note)
}

fn check_texts_height(title: &egui::Galley, note: Option<&Arc<egui::Galley>>) -> f32 {
    title.size().y + note.map_or(0.0, |note| note.size().y + 1.0)
}

/// Toggles a check row when it was clicked and paints it, its text starting at `top`.
#[allow(clippy::too_many_arguments)]
fn paint_check_row(
    ui: &Ui,
    rect: Rect,
    top: f32,
    response: Response,
    checked: &mut bool,
    text: &str,
    title: Arc<egui::Galley>,
    note: Option<Arc<egui::Galley>>,
) -> Response {
    let p = palette(ui.ctx());
    if response.clicked() {
        *checked = !*checked;
    }
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, ui.is_enabled(), *checked, text));
    if ui.is_rect_visible(rect) {
        let first_line = title.rows.first().map_or(CHECK_SIZE, |row| row.height());
        let check = Rect::from_min_size(pos2(rect.left(), top + (first_line - CHECK_SIZE) / 2.0), Vec2::splat(CHECK_SIZE));
        paint_check(ui, check, *checked, response.hovered(), p.accent);
        focus_ring(ui, check, &response, 6);
        let left = rect.left() + CHECK_SIZE + 12.0;
        let title_height = title.size().y;
        ui.painter().galley(pos2(left, top), title, p.text);
        if let Some(note) = note {
            ui.painter().galley(pos2(left, top + title_height + 1.0), note, p.muted);
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// An on/off switch. `label` is what a screen reader says.
pub fn switch(ui: &mut Ui, on: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Switch, ui.is_enabled(), on, label));
    if ui.is_rect_visible(rect) {
        let p = palette(ui.ctx());
        let t = ui.ctx().animate_bool_responsive(response.id, on);
        let painter = ui.painter();
        let radius = CornerRadius::same(11);
        painter.rect_filled(rect, radius, p.subtle.lerp_to_gamma(p.accent, t));
        if t < 1.0 {
            let edge = if response.hovered() { p.text } else { p.muted };
            painter.rect_stroke(rect, radius, Stroke::new(1.0, edge.gamma_multiply(1.0 - t)), StrokeKind::Inside);
        }
        let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
        let knob = p.muted.lerp_to_gamma(Color32::WHITE, t);
        painter.circle_filled(pos2(x, rect.center().y), 6.0 + t, knob);
        focus_ring(ui, rect, &response, 11);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A row of choices of which one is picked, such as the tabs. Returns the one clicked, if it isn't picked yet.
pub fn segmented(ui: &mut Ui, id: Id, options: &[&str], picked: usize) -> Option<usize> {
    let p = palette(ui.ctx());
    let height = 34.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, CornerRadius::same(9), p.subtle);
    let width = (rect.width() - 6.0) / options.len().max(1) as f32;
    let mut clicked = None;
    for (index, option) in options.iter().enumerate() {
        let segment = Rect::from_min_size(pos2(rect.left() + 3.0 + width * index as f32, rect.top() + 3.0), vec2(width, height - 6.0));
        let response = ui.interact(segment, id.with(index), Sense::click());
        response.widget_info(|| WidgetInfo::selected(Role::Tab, ui.is_enabled(), index == picked, option));
        let chosen = index == picked;
        if chosen {
            painter.rect_filled(segment, CornerRadius::same(7), p.selected);
            painter.rect_stroke(segment, CornerRadius::same(7), Stroke::new(1.0, p.border), StrokeKind::Inside);
        } else if response.hovered() {
            painter.rect_filled(segment, CornerRadius::same(7), p.subtle_hover);
        }
        let font = if chosen { bold(13.0) } else { regular(13.0) };
        painter.text(segment.center(), Align2::CENTER_CENTER, option, font, if chosen { p.text } else { p.muted });
        focus_ring(ui, segment, &response, 7);
        if response.clicked() && !chosen {
            clicked = Some(index);
        }
        if !chosen {
            response.on_hover_cursor(CursorIcon::PointingHand);
        }
    }
    clicked
}

/// A thin bar that fills up, as far as `fraction`.
pub fn progress(ui: &mut Ui, id: Id, fraction: f32, color: Color32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    let shown = ui.ctx().animate_value_with_time(id, fraction.clamp(0.0, 1.0), 0.25);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), p.subtle);
    if shown > 0.0 {
        let filled = Rect::from_min_size(rect.min, vec2((rect.width() * shown).max(8.0), rect.height()));
        painter.rect_filled(filled, CornerRadius::same(4), color);
    }
}

/// A one-line text field with a hint while it is empty.
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    let p = palette(ui.ctx());
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(p.muted))
            .font(regular(13.0))
            .margin(vec2(10.0, 8.0))
            .desired_width(f32::INFINITY),
    )
}

// --- Words for dates ----------------------------------------------------------------------------------------

/// 'Nov 3'.
pub fn day(date: NaiveDate) -> String {
    date.format("%b %-d").to_string()
}

/// 'Nov 3', or 'Nov 3, 2025' if that isn't this year.
pub fn on(date: NaiveDate, today: NaiveDate) -> String {
    if date.year() == today.year() { day(date) } else { format!("{}, {}", day(date), date.year()) }
}

/// 'today', 'yesterday', '5 days ago', '3 weeks ago' or '4 months ago'.
pub fn ago(date: NaiveDate, today: NaiveDate) -> String {
    match (today - date).num_days() {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        days @ 2..14 => format!("{days} days ago"),
        days @ 14..60 => format!("{} weeks ago", days / 7),
        days => format!("{} months ago", days / 30),
    }
}

/// 'Mon, Nov 3 at 09:00'.
pub fn when(date: NaiveDate, hour: u32) -> String {
    format!("{}, {} at {:02}:00", date.format("%a"), day(date), hour % 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn dates_read_naturally() {
        let today = date(2026, 11, 8);
        assert_eq!(day(date(2026, 11, 3)), "Nov 3");
        assert_eq!(on(date(2026, 11, 3), today), "Nov 3");
        assert_eq!(on(date(2025, 12, 30), today), "Dec 30, 2025");
        assert_eq!(when(date(2026, 11, 9), 9), "Mon, Nov 9 at 09:00");
    }

    #[test]
    fn ago_rounds_down_to_the_larger_unit() {
        let today = date(2026, 11, 8);
        let back = |days: i64| ago(today - chrono::Duration::days(days), today);
        assert_eq!(ago(today + chrono::Duration::days(1), today), "today");
        assert_eq!(back(0), "today");
        assert_eq!(back(1), "yesterday");
        assert_eq!(back(13), "13 days ago");
        assert_eq!(back(14), "2 weeks ago");
        assert_eq!(back(59), "8 weeks ago");
        assert_eq!(back(60), "2 months ago");
        assert_eq!(back(400), "13 months ago");
    }

    #[test]
    fn both_themes_keep_text_readable() {
        // WCAG's 4.5:1 for body text, on the cards and on the window's background.
        for p in [&LIGHT, &DARK] {
            for background in [p.card, p.bg, p.card_done] {
                assert!(contrast(p.text, background) >= 7.0);
                assert!(contrast(p.muted, background) >= 4.5, "{:?} on {:?}", p.muted, background);
            }
            assert!(contrast(p.success_text, p.card) >= 4.5);
            assert!(contrast(Color32::WHITE, p.accent) >= 3.0);
            assert!(contrast(Color32::WHITE, p.danger) >= 3.0);
        }
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        fn luminance(color: Color32) -> f32 {
            let channel = |c: u8| {
                let c = f32::from(c) / 255.0;
                if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}
