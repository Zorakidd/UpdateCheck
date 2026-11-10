//! The checklist tab: a card for each step, ticked off with a click anywhere on it, that says what the step
//! has waiting wherever the app can find that out.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::NaiveDate;
use eframe::egui;
use egui::text::LayoutJob;
use egui::{
    Color32, CornerRadius, CursorIcon, Galley, Rect, Response, Role, Sense, Stroke, StrokeKind, TextFormat, Ui,
    Vec2, WidgetInfo, pos2, vec2,
};

use super::{Action, App};
use crate::graphics::{Card, DriverStatus, Vendor};
use crate::pending::{self, Status};
use crate::state::{self, State, Step};
use crate::system;
use crate::ui::{self, Look, Palette};

/// Where a card's text begins, right of its check box.
const LEFT: f32 = 50.0;
/// The room above and below a card's text.
const PAD: f32 = 14.0;
/// The room between two cards.
const GAP: f32 = 10.0;
/// Less free space than this on the system drive is worth freeing some up.
const LOW_SPACE: u64 = 20 * 1024 * 1024 * 1024;
/// How many of the upgrades waiting a card names before it only counts the rest.
const ITEMS_SHOWN: usize = 5;
/// The steps that tick themselves off once their lookup finds nothing waiting. For Windows Update that includes
/// no restart still to make.
const TICKED_WHEN_NOTHING_WAITS: &[&str] = &[state::WINDOWS_UPDATE_STEP, state::DRIVERS_STEP, state::WINGET_STEP];

/// How a part of a card's line under its title reads: as a plain fact, as something to do, or as good news.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Plain,
    ToDo,
    Good,
}

impl Tone {
    fn color(self, p: &Palette) -> Color32 {
        match self {
            Tone::Plain => p.muted,
            Tone::ToDo => p.accent,
            Tone::Good => p.success_text,
        }
    }
}

/// What a graphics card's row says, and what its button does.
struct DriverRow {
    name: String,
    status: String,
    tone: Tone,
    /// Whether the driver needs installing, which makes the row's button the one to press.
    urgent: bool,
    button: String,
    vendor: Vendor,
    url: String,
}

/// A graphics card's row, laid out.
struct DriverLines {
    button: Vec2,
    name: Arc<Galley>,
    status: Arc<Galley>,
    height: f32,
}

/// The steps to tick off by themselves now: those of `TICKED_WHEN_NOTHING_WAITS` the checklist has, whose lookup
/// found nothing waiting, and that aren't ticked yet or were unticked by hand this week.
fn steps_to_tick(
    state: &State,
    unticked: &BTreeSet<(Option<NaiveDate>, String)>,
    status: impl Fn(&str) -> Status,
) -> Vec<&'static str> {
    TICKED_WHEN_NOTHING_WAITS
        .iter()
        .copied()
        .filter(|id| state.steps.iter().any(|step| step.id == *id))
        .filter(|id| !state.done.contains(*id) && !unticked.contains(&(state.cycle_start, (*id).to_owned())))
        .filter(|id| status(id) == Status::NothingPending)
        .collect()
}

impl App {
    /// Ticks off a step whose lookup found nothing waiting, such as Desktop apps when winget has no upgrades.
    pub(super) fn tick_steps_with_nothing_waiting(&mut self) {
        let ids = steps_to_tick(&self.state, &self.unticked, |id| self.pending.status(id));
        if ids.is_empty() {
            return;
        }
        let now = state::local_now();
        for id in ids {
            self.state.set_done(id, true, &now);
        }
        self.save();
    }

    pub(super) fn checklist(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        if self.state.steps.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui::text(ui, "No steps yet. Add some in Settings.", ui::regular(13.0), p.muted);
            });
            return;
        }
        let today = state::local_now().date_naive();
        // A copy, since drawing a card fills the app's caches, such as which graphics apps are installed.
        let steps = self.state.steps.clone();
        for (index, step) in steps.iter().enumerate() {
            if index > 0 {
                ui.add_space(GAP);
            }
            if step.id == state::GRAPHICS_STEP {
                self.graphics_card(ui, step, today);
            } else {
                self.step_card(ui, step, today);
            }
        }
    }

    fn step_card(&mut self, ui: &mut Ui, step: &Step, today: NaiveDate) {
        let p = ui::palette(ui.ctx());
        let done = self.state.done.contains(&step.id);
        let width = ui.available_width();
        let has_button = !step.action.trim().is_empty() || step.id == state::APPS_STEP;
        let button = has_button.then(|| ui::button_size(ui, "Open", Look::Secondary));
        let text_width = (width - LEFT - 16.0 - button.map_or(0.0, |size| size.x + 16.0)).max(80.0);
        let painter = ui.painter().clone();

        let title = painter.layout(step.title.clone(), ui::bold(15.0), if done { p.muted } else { p.text }, text_width);
        let mut lines = Vec::new();
        if !step.detail.trim().is_empty() {
            lines.push(painter.layout(step.detail.trim().to_owned(), ui::regular(12.0), p.muted, text_width));
        }
        let meta = self.meta(step, today);
        if !meta.is_empty() {
            lines.push(painter.layout_job(meta_job(meta, p, text_width)));
        }
        if let Status::Pending(_) = self.pending.status(&step.id) {
            let items = self.pending.items(&step.id);
            for item in items.iter().take(ITEMS_SHOWN) {
                lines.push(painter.layout(format!("\u{2022} {item}"), ui::regular(12.0), p.text, text_width));
            }
            if items.len() > ITEMS_SHOWN {
                let more = format!("+{} more", items.len() - ITEMS_SHOWN);
                lines.push(painter.layout(more, ui::regular(12.0), p.muted, text_width));
            }
        }
        let content = title.size().y + lines.iter().map(|line| 2.0 + line.size().y).sum::<f32>();
        let height = (content + 2.0 * PAD).max(button.map_or(0.0, |size| size.y + 2.0 * PAD));

        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        let response = self.tick_on_click(ui, response, step, done);
        paint_card(ui, rect, &response, done);
        paint_tick(ui, rect, first_row(&title), done, response.hovered());
        let x = rect.left() + LEFT;
        let mut y = rect.top() + PAD;
        let title_height = title.size().y;
        painter.galley(pos2(x, y), title, p.text);
        y += title_height;
        for line in lines {
            y += 2.0;
            let line_height = line.size().y;
            painter.galley(pos2(x, y), line, p.text);
            y += line_height;
        }
        if let Some(size) = button {
            let place = Rect::from_min_size(pos2(rect.right() - 16.0 - size.x, rect.center().y - size.y / 2.0), size);
            let id = ui.scope_id().with(("open", &step.id));
            if ui::button_at(ui, place, id, "Open", Look::Secondary, true).clicked() {
                self.actions.push(Action::OpenStep(step.id.clone()));
            }
        }
    }

    /// The parts of the line under a step's title: what it has waiting, and when it was last done.
    fn meta(&self, step: &Step, today: NaiveDate) -> Vec<(String, Tone)> {
        let mut parts = Vec::new();
        let id = step.id.as_str();
        if self.pending.handles(id) {
            match self.pending.status(id) {
                Status::Checking => parts.push(("checking\u{2026}".to_owned(), Tone::Plain)),
                Status::Pending(count) => parts.push((pending::describe(id, count), Tone::ToDo)),
                Status::NothingPending => parts.push(("nothing waiting".to_owned(), Tone::Good)),
                Status::RestartPending => parts.push(("restart to finish installing".to_owned(), Tone::ToDo)),
                // Nothing to say that would help; the step is there to be looked into by hand anyway.
                Status::CheckFailed => {}
            }
            if let Some(date) = self.pending.last_installed(id) {
                parts.push((format!("last installed {}", ui::on(date, today)), Tone::Plain));
            }
        } else if id == state::STORAGE_STEP {
            if let Some((drive, free, _)) = &self.storage {
                let low = *free < LOW_SPACE;
                let text = format!("{}{} free on {drive}", if low { "only " } else { "" }, system::size(*free));
                parts.push((text, if low { Tone::ToDo } else { Tone::Plain }));
            }
        } else if id == state::APPS_STEP
            && let Some(part) = self.apps_meta()
        {
            parts.push(part);
        }
        if let Some(date) = self.state.last_done.get(id) {
            parts.push((format!("last done {}", ui::ago(*date, today)), Tone::Plain));
        }
        parts
    }

    /// The card's click ticks the step off, or back on, and a screen reader hears it as a check box.
    fn tick_on_click(&mut self, ui: &Ui, response: Response, step: &Step, done: bool) -> Response {
        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, ui.is_enabled(), done, &step.title));
        if response.clicked() {
            self.actions.push(Action::Toggle(step.id.clone()));
        }
        response.on_hover_cursor(CursorIcon::PointingHand)
    }

    /// The graphics step's card, with a row for each graphics card and its driver.
    fn graphics_card(&mut self, ui: &mut Ui, step: &Step, today: NaiveDate) {
        let p = ui::palette(ui.ctx());
        let done = self.state.done.contains(&step.id);
        let width = ui.available_width();
        let full = (width - LEFT - 16.0).max(80.0);
        let painter = ui.painter().clone();
        let cards = self.drivers.cards.clone();
        let rows: Vec<DriverRow> = cards.iter().map(|card| self.driver_row(card)).collect();

        let title = painter.layout(step.title.clone(), ui::bold(15.0), if done { p.muted } else { p.text }, full);
        let mut lines = Vec::new();
        if !step.detail.trim().is_empty() {
            lines.push(painter.layout(step.detail.trim().to_owned(), ui::regular(12.0), p.muted, full));
        }
        if let Some(date) = self.state.last_done.get(&step.id) {
            let text = format!("Last done {}", ui::ago(*date, today));
            lines.push(painter.layout(text, ui::regular(12.0), p.muted, full));
        }
        if cards.is_empty() {
            let text = "No NVIDIA, AMD or Intel graphics found.".to_owned();
            lines.push(painter.layout(text, ui::regular(12.0), p.muted, full));
        }
        let laid: Vec<DriverLines> = rows
            .iter()
            .map(|row| {
                let button = ui::button_size(ui, &row.button, look(row.urgent));
                let text_width = (full - button.x - 16.0).max(80.0);
                let name = painter.layout(row.name.clone(), ui::bold(13.0), p.text, text_width);
                let font = if row.urgent { ui::bold(12.0) } else { ui::regular(12.0) };
                let status = painter.layout(row.status.clone(), font, row.tone.color(p), text_width);
                let height = (name.size().y + 2.0 + status.size().y).max(button.y);
                DriverLines { button, name, status, height }
            })
            .collect();
        let head = title.size().y + lines.iter().map(|line| 2.0 + line.size().y).sum::<f32>();
        let content = head + laid.iter().map(|row| 10.0 + row.height).sum::<f32>();

        let (rect, response) = ui.allocate_exact_size(vec2(width, content + 2.0 * PAD), Sense::click());
        let response = self.tick_on_click(ui, response, step, done);
        paint_card(ui, rect, &response, done);
        paint_tick(ui, rect, first_row(&title), done, response.hovered());
        let x = rect.left() + LEFT;
        let mut y = rect.top() + PAD;
        let title_height = title.size().y;
        painter.galley(pos2(x, y), title, p.text);
        y += title_height;
        for line in lines {
            y += 2.0;
            let line_height = line.size().y;
            painter.galley(pos2(x, y), line, p.text);
            y += line_height;
        }
        for (index, (row, lines)) in rows.iter().zip(laid).enumerate() {
            y += 10.0;
            let text_height = lines.name.size().y + 2.0 + lines.status.size().y;
            let mut text_y = y + (lines.height - text_height) / 2.0;
            let name_height = lines.name.size().y;
            painter.galley(pos2(x, text_y), lines.name, p.text);
            text_y += name_height + 2.0;
            painter.galley(pos2(x, text_y), lines.status, p.text);
            let size = lines.button;
            let place = Rect::from_min_size(pos2(rect.right() - 16.0 - size.x, y + (lines.height - size.y) / 2.0), size);
            let id = ui.scope_id().with(("driver", index));
            if ui::button_at(ui, place, id, &row.button, look(row.urgent), true).clicked() {
                self.actions.push(Action::OpenDriver { vendor: row.vendor, url: row.url.clone() });
            }
            y += lines.height;
        }
    }

    fn driver_row(&mut self, card: &Card) -> DriverRow {
        let version = card.version();
        let app = self.vendor_app(card.vendor);
        let status = self.drivers.status(card);
        let (text, tone, urgent) = match &status {
            DriverStatus::Checking => (format!("Driver {version} \u{b7} Checking for updates\u{2026}"), Tone::Plain, false),
            DriverStatus::UpToDate(_) => (format!("Driver {version} \u{b7} Up to date"), Tone::Good, false),
            DriverStatus::UpdateAvailable(release) => {
                (format!("Driver {version} \u{b7} Update available: {}", release.version), Tone::ToDo, true)
            }
            DriverStatus::CheckFailed => (format!("Driver {version} \u{b7} Couldn't check for updates"), Tone::Plain, false),
            DriverStatus::NoDriver => ("No driver installed".to_owned(), Tone::ToDo, true),
            DriverStatus::NotChecked => {
                let place = if app.is_some() { "in the app" } else { "on the website" };
                (format!("Driver {version} \u{b7} Check for updates {place}"), Tone::Plain, false)
            }
        };
        let url = match &status {
            DriverStatus::UpToDate(release) | DriverStatus::UpdateAvailable(release) => release.url.clone(),
            _ => card.vendor.download_page().to_owned(),
        };
        let button = app.map_or_else(|| "Download driver".to_owned(), |app| format!("Open {}", app.name));
        DriverRow { name: card.name.clone(), status: text, tone, urgent, button, vendor: card.vendor, url }
    }
}

fn look(urgent: bool) -> Look {
    if urgent { Look::Primary } else { Look::Secondary }
}

/// The height of a title's first line, which the check box is centered on.
fn first_row(title: &Galley) -> f32 {
    title.rows.first().map_or(ui::CHECK_SIZE, |row| row.height())
}

fn paint_card(ui: &Ui, rect: Rect, response: &Response, done: bool) {
    let p = ui::palette(ui.ctx());
    let fill = if done { p.card_done } else { p.card };
    let edge = if response.hovered() { p.muted } else { p.border };
    ui.painter().rect(rect, CornerRadius::same(12), fill, Stroke::new(1.0, edge), StrokeKind::Inside);
    ui::focus_ring(ui, rect, response, 12);
}

fn paint_tick(ui: &Ui, rect: Rect, first_row: f32, done: bool, hovered: bool) {
    let top = rect.top() + PAD + (first_row - ui::CHECK_SIZE) / 2.0;
    let check = Rect::from_min_size(pos2(rect.left() + 16.0, top), Vec2::splat(ui::CHECK_SIZE));
    ui::paint_check(ui, check, done, hovered, ui::palette(ui.ctx()).success);
}

/// The parts in one line, joined by dots, the first one capitalized.
fn meta_job(parts: Vec<(String, Tone)>, p: &Palette, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let format = |color: Color32| TextFormat { font_id: ui::regular(12.0), color, ..Default::default() };
    for (index, (text, tone)) in parts.into_iter().enumerate() {
        if index == 0 {
            job.append(&capitalized(&text), 0.0, format(tone.color(p)));
        } else {
            job.append(" \u{b7} ", 0.0, format(p.muted));
            job.append(&text, 0.0, format(tone.color(p)));
        }
    }
    job.wrap.max_width = width;
    job
}

pub(super) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_part_starts_with_a_capital() {
        assert_eq!(capitalized("checking\u{2026}"), "Checking\u{2026}");
        assert_eq!(capitalized("3 updates waiting"), "3 updates waiting");
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn steps_tick_themselves_off_when_their_lookup_finds_nothing() {
        let mut state = State { cycle_start: NaiveDate::from_ymd_opt(2026, 11, 9), ..State::default() };
        let mut unticked = BTreeSet::new();
        assert_eq!(
            steps_to_tick(&state, &unticked, |_| Status::NothingPending),
            [state::WINDOWS_UPDATE_STEP, state::DRIVERS_STEP, state::WINGET_STEP]
        );
        // Only a lookup that found nothing ticks a step, not one that found something waiting or a restart still
        // to make, or that failed or is still running.
        for status in [Status::Pending(2), Status::RestartPending, Status::CheckFailed, Status::Checking] {
            assert!(steps_to_tick(&state, &unticked, |_| status).is_empty());
        }
        // From here on, only winget finds nothing.
        let nothing = |id: &str| if id == state::WINGET_STEP { Status::NothingPending } else { Status::Pending(1) };
        assert_eq!(steps_to_tick(&state, &unticked, nothing), [state::WINGET_STEP]);
        // Unticked by hand, it stays unticked that week, and ticks itself off again the next.
        unticked.insert((state.cycle_start, state::WINGET_STEP.to_owned()));
        assert!(steps_to_tick(&state, &unticked, nothing).is_empty());
        state.cycle_start = NaiveDate::from_ymd_opt(2026, 11, 16);
        assert_eq!(steps_to_tick(&state, &unticked, nothing), [state::WINGET_STEP]);
        state.done.insert(state::WINGET_STEP.to_owned());
        assert!(steps_to_tick(&state, &unticked, nothing).is_empty());
        state.steps.retain(|step| step.id != state::WINGET_STEP);
        state.done.clear();
        assert!(steps_to_tick(&state, &unticked, nothing).is_empty());
    }
}
