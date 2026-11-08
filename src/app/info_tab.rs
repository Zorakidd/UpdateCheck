//! The PC info tab: what is in the PC and how up to date Windows and its firmware are, in plain words, with the
//! details that matter when buying parts or asking for help a tick away.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Margin, Rect, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2};

use super::{Action, App, Task, finished};
use crate::pcinfo::{self, Row, Section};
use crate::ui::{self, Look};
use crate::{applog, pending, state};

/// How long the copy button says it copied.
const COPIED_FOR: Duration = Duration::from_millis(1500);
/// The PC's parts don't change while it runs, apart from drives plugged in, so the tab reads them again only
/// after this long. Reading them can wake a sleeping hard drive.
const READ_AGAIN_AFTER: Duration = Duration::from_secs(10 * 60);
/// The width of the labels on the left of each card.
const LABEL_WIDTH: f32 = 104.0;

#[derive(Default)]
pub(super) struct InfoPage {
    sections: Vec<Section>,
    /// Whether Windows waits for a restart to finish installing updates.
    restart: bool,
    loading: Option<Task<Vec<Section>>>,
    read_at: Option<Instant>,
    details: bool,
    pub(super) copied_until: Option<Instant>,
}

impl App {
    pub(super) fn info_visited(&mut self) {
        // Asked each time, as it changes once updates are installed, and it takes microseconds.
        self.info.restart = pcinfo::restart_pending();
        if self.info.loading.is_some() || self.info.read_at.is_some_and(|at| at.elapsed() < READ_AGAIN_AFTER) {
            return;
        }
        let today = state::local_now().date_naive();
        // What Windows Update says is filled in when the tab is drawn, since it keeps changing.
        self.info.loading = Some(Task::spawn(&self.ctx, "pc info", move || pcinfo::gather("\u{2026}", today)));
    }

    pub(super) fn collect_info(&mut self) {
        if let Some(result) = finished(&mut self.info.loading) {
            match result {
                Some(sections) => self.info.sections = sections,
                None => applog::error("pcinfo", "Reading the PC's details stopped unexpectedly."),
            }
            self.info.read_at = Some(Instant::now());
        }
        if self.info.copied_until.is_some_and(|until| Instant::now() >= until) {
            self.info.copied_until = None;
        }
    }

    /// The sections as read, with what Windows Update says now.
    fn info_sections(&self) -> Vec<Section> {
        let updates = super::windows_update_text(self.pending.status(pending::WINDOWS_UPDATE), self.info.restart);
        let mut sections = self.info.sections.clone();
        for section in sections.iter_mut().filter(|section| section.title == "Windows") {
            for row in section.rows.iter_mut().filter(|row| row.label == "Updates") {
                row.value.clone_from(&updates);
            }
        }
        sections
    }

    pub(super) fn info_tab(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let sections = self.info_sections();

        // The details box on the left, and the copy button on the right, centered on it.
        let size = ui::button_size(ui, "Copy to clipboard", Look::Secondary);
        let width = ui.available_width();
        let top = ui.cursor().min;
        let left = UiBuilder::new().max_rect(Rect::from_min_size(top, vec2(width - size.x - 16.0, f32::INFINITY)));
        let mut details = self.info.details;
        let note = "Memory modules, processor cache, graphics memory and every drive";
        let used = ui.scope_builder(left, |ui| ui::check_row(ui, &mut details, "Show details", note)).response.rect;
        self.info.details = details;
        let place = Rect::from_min_size(pos2(top.x + width - size.x, used.center().y - size.y / 2.0), size);
        let label = if self.info.copied_until.is_some() { "Copied" } else { "Copy to clipboard" };
        let id = ui.scope_id().with("copy-info");
        let copy = ui::button_at(ui, place, id, label, Look::Secondary, !sections.is_empty());
        if copy.on_hover_text("Copies what the tab shows, to paste into a message or a support request").clicked() {
            ui.ctx().copy_text(pcinfo::as_text(&sections, details));
            self.info.copied_until = Some(Instant::now() + COPIED_FOR);
        }
        // The button may stand out below the box.
        ui.add_space((place.bottom() - used.bottom()).max(0.0));

        if sections.is_empty() {
            ui.add_space(32.0);
            let reading = self.info.loading.is_some() || self.info.read_at.is_none();
            let text = if reading { "Reading this PC's details\u{2026}" } else { "Couldn't read this PC's details." };
            ui.vertical_centered(|ui| ui::text(ui, text, ui::regular(13.0), p.muted));
            return;
        }
        let mut open = None;
        for (index, section) in sections.iter().filter(|section| section.is_visible(details)).enumerate() {
            ui::section(ui, section.title, if index == 0 { 10.0 } else { 18.0 });
            ui::card(p).inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 14 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (index, row) in section.visible_rows(details).enumerate() {
                    if index > 0 {
                        ui.add_space(7.0);
                    }
                    info_row(ui, row);
                }
                if let Some((label, target)) = &section.action {
                    ui.add_space(12.0);
                    if ui::button(ui, label, Look::Secondary).clicked() {
                        open = Some(target.clone());
                    }
                }
            });
        }
        if let Some(target) = open {
            self.actions.push(Action::OpenUrl(target));
        }
    }
}

/// A label on the left, in the muted color, and its value beside it, perhaps over several lines.
fn info_row(ui: &mut Ui, row: &Row) {
    let p = ui::palette(ui.ctx());
    let width = ui.available_width();
    let painter = ui.painter().clone();
    let label = painter.layout(row.label.clone(), ui::regular(12.0), p.muted, LABEL_WIDTH - 10.0);
    let value = painter.layout(row.value.clone(), ui::regular(13.0), p.text, (width - LABEL_WIDTH).max(80.0));
    // The label sits a pixel lower, so both share a baseline.
    let height = value.size().y.max(label.size().y + 1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("{}: {}", row.label, row.value)));
    painter.galley(pos2(rect.left(), rect.top() + 1.0), label, p.muted);
    painter.galley(pos2(rect.left() + LABEL_WIDTH, rect.top()), value, p.text);
}
