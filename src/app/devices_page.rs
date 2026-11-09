//! The page of connected devices: everything plugged into the PC or paired with it, by kind, with how each one
//! is connected and whether Windows has a problem with it. The list follows along when something is plugged in
//! or out, and each device, or the whole list, can be copied. The PC info tab opens it.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Rect, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2};

use super::{Action, App, Page, Task, finished};
use crate::applog;
use crate::devices::{self, Device, Kind};
use crate::ui::{self, Look};

/// How often the page looks whether a device was plugged in or out.
const POLL: Duration = Duration::from_secs(1);
/// How often it reads the list again anyway, as a Bluetooth device connecting changes no device Windows has.
const READ_AGAIN: Duration = Duration::from_secs(5);
/// How long a copy button says it copied.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// What was copied last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Copied {
    All,
    /// The device at this place in the list.
    Device(usize),
}

pub(super) struct DevicesPage {
    /// The devices as last listed, or why Windows couldn't list them.
    list: Option<Result<Vec<Device>, String>>,
    loading: Option<Task<Result<Vec<Device>, String>>>,
    /// What said which devices there were when they were last listed: see `devices::signal`.
    signal: u32,
    polled: Instant,
    read: Instant,
    /// What was copied, and until when its button says so.
    pub(super) copied: Option<(Copied, Instant)>,
}

impl DevicesPage {
    pub(super) fn new() -> DevicesPage {
        DevicesPage { list: None, loading: None, signal: 0, polled: Instant::now(), read: Instant::now(), copied: None }
    }

    fn copied(&self, what: Copied) -> bool {
        self.copied.is_some_and(|(copied, _)| copied == what)
    }
}

impl App {
    pub(super) fn devices_visited(&mut self) {
        self.load_devices();
    }

    pub(super) fn devices_focus_gained(&mut self) {
        if self.page == Page::Devices {
            self.load_devices();
        }
    }

    /// Lists the devices again, in the background. It takes milliseconds and touches no drive.
    fn load_devices(&mut self) {
        if self.devices.loading.is_some() {
            return;
        }
        let now = Instant::now();
        self.devices.signal = devices::signal();
        (self.devices.polled, self.devices.read) = (now, now);
        self.devices.loading = Some(Task::spawn(&self.ctx, "devices", devices::connected));
    }

    pub(super) fn devices_poll(&mut self) {
        let page = &self.devices;
        if self.page != Page::Devices || page.loading.is_some() || page.polled.elapsed() < POLL {
            return;
        }
        self.devices.polled = Instant::now();
        if devices::signal() != self.devices.signal || self.devices.read.elapsed() >= READ_AGAIN {
            self.load_devices();
        }
    }

    pub(super) fn devices_poll_at(&self) -> Instant {
        self.devices.polled + POLL
    }

    pub(super) fn collect_devices(&mut self) {
        if self.devices.copied.is_some_and(|(_, until)| Instant::now() >= until) {
            self.devices.copied = None;
        }
        let Some(result) = finished(&mut self.devices.loading) else { return };
        let result = result.unwrap_or_else(|| Err("Listing the devices stopped unexpectedly.".to_owned()));
        // Said once, not every few seconds while the page is open.
        if let Err(error) = &result
            && !matches!(&self.devices.list, Some(Err(before)) if before == error)
        {
            applog::error("devices", error);
        }
        self.devices.list = Some(result);
    }

    pub(super) fn devices_page(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let intro = "What is plugged into this PC, or paired with it and connected, as Windows sees it. The list \
                     follows along when something is plugged in or out.";
        let shown = self.devices.list.as_ref().and_then(|list| list.as_ref().ok()).filter(|list| !list.is_empty());
        let mut copy = None;

        // The intro on the left, and the button that copies everything on the right, centered on it, as on the
        // PC info tab.
        let size = ui::button_size(ui, "Copy all", Look::Secondary);
        let width = ui.available_width();
        let top = ui.cursor().min;
        let left = UiBuilder::new().max_rect(Rect::from_min_size(top, vec2(width - size.x - 16.0, f32::INFINITY)));
        let used = ui.scope_builder(left, |ui| ui::text(ui, intro, ui::regular(13.0), p.muted)).response.rect;
        let place = Rect::from_min_size(pos2(top.x + width - size.x, used.center().y - size.y / 2.0), size);
        let label = if self.devices.copied(Copied::All) { "Copied" } else { "Copy all" };
        let id = ui.scope_id().with("copy-devices");
        let button = ui::button_at(ui, place, id, label, Look::Secondary, shown.is_some());
        if button.on_hover_text("Copies the whole list, to paste into a message or a support request").clicked()
            && let Some(list) = shown
        {
            copy = Some((Copied::All, devices::as_text(list)));
        }
        // The button may stand out below the intro.
        ui.add_space((place.bottom() - used.bottom()).max(0.0) + 14.0);

        match &self.devices.list {
            None => {
                ui::text(ui, "Looking for devices\u{2026}", ui::regular(13.0), p.muted);
            }
            Some(Err(error)) => {
                ui::card(p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui::text(ui, "Couldn't list the devices", ui::bold(14.0), p.danger);
                    ui.add_space(2.0);
                    ui::text(ui, error, ui::regular(12.0), p.muted);
                });
            }
            Some(Ok(list)) if list.is_empty() => {
                ui::card(p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui::text(ui, "Nothing is plugged in or paired", ui::bold(14.0), p.text);
                    ui.add_space(2.0);
                    let text = "A device shows up here as soon as it is plugged in, or paired and connected.";
                    ui::text(ui, text, ui::regular(12.0), p.muted);
                });
            }
            Some(Ok(list)) => {
                let kinds = Kind::ALL.iter().filter(|kind| list.iter().any(|device| device.kind == **kind));
                for (index, kind) in kinds.enumerate() {
                    ui::section(ui, kind.title(), if index == 0 { 0.0 } else { 18.0 });
                    ui::card(p).inner_margin(egui::Margin { left: 16, right: 12, top: 12, bottom: 12 }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let devices = list.iter().enumerate().filter(|(_, device)| device.kind == *kind);
                        for (row, (place, device)) in devices.enumerate() {
                            if row > 0 {
                                ui.add_space(8.0);
                            }
                            if device_row(ui, device, place, self.devices.copied(Copied::Device(place))) {
                                copy = Some((Copied::Device(place), devices::line(device)));
                            }
                        }
                    });
                }
            }
        }
        if let Some((what, text)) = copy {
            ui.ctx().copy_text(text);
            self.devices.copied = Some((what, Instant::now() + COPIED_FOR));
        }

        ui.add_space(18.0);
        let mut open = None;
        ui.horizontal(|ui| {
            let settings = ui::button(ui, "Bluetooth & devices", Look::Secondary);
            if settings.on_hover_text("Pair a device, or change how a mouse, a pen or a printer works").clicked() {
                open = Some("ms-settings:bluetooth");
            }
            ui.add_space(4.0);
            let manager = ui::button(ui, "Device Manager", Look::Secondary);
            if manager.on_hover_text("Every device in and around the PC, with its driver").clicked() {
                open = Some("devmgmt.msc");
            }
        });
        if let Some(target) = open {
            self.actions.push(Action::OpenUrl(target.to_owned()));
        }
    }
}

/// A device's name on the left with what is wrong with it below, if anything, how it is connected to the right of
/// it, and a button that copies it at the end. Returns whether the button was clicked.
fn device_row(ui: &mut Ui, device: &Device, place: usize, copied: bool) -> bool {
    let p = ui::palette(ui.ctx());
    let width = ui.available_width();
    let painter = ui.painter().clone();
    // As wide as its longer label, so it doesn't change size when it says it copied.
    let button = ui::button_size(ui, "Copied", Look::Secondary);
    let connection = device.connection.map(|connection| painter.layout_no_wrap(connection.name().to_owned(), ui::regular(12.0), p.muted));
    let beside = button.x + connection.as_ref().map_or(0.0, |text| text.size().x + 16.0) + 16.0;
    let text_width = (width - beside).max(80.0);
    let name = painter.layout(device.name.clone(), ui::regular(13.0), p.text, text_width);
    let problem = device.problem.as_ref().map(|problem| painter.layout(problem.clone(), ui::regular(12.0), p.danger, text_width));
    let text_height = name.size().y + problem.as_ref().map_or(0.0, |text| 2.0 + text.size().y);
    let (rect, response) = ui.allocate_exact_size(vec2(width, text_height.max(button.y)), Sense::hover());
    response.widget_info(|| {
        let parts = [Some(device.name.as_str()), device.connection.map(|connection| connection.name()), device.problem.as_deref()];
        WidgetInfo::labeled(Role::Label, true, parts.into_iter().flatten().collect::<Vec<_>>().join(", "))
    });
    let text_top = rect.top() + (rect.height() - text_height) / 2.0;
    let name_height = name.size().y;
    painter.galley(pos2(rect.left(), text_top), name, p.text);
    if let Some(problem) = problem {
        painter.galley(pos2(rect.left(), text_top + name_height + 2.0), problem, p.danger);
    }
    let button_rect = Rect::from_min_size(pos2(rect.right() - button.x, rect.center().y - button.y / 2.0), button);
    if let Some(connection) = connection {
        // Centered on the name's line, a pixel lower so it shares the name's baseline.
        let y = text_top + (name_height - connection.size().y) / 2.0 + 1.0;
        painter.galley(pos2(button_rect.left() - 16.0 - connection.size().x, y), connection, p.muted);
    }
    let id = ui.scope_id().with(("copy-device", place));
    let label = if copied { "Copied" } else { "Copy" };
    let copy = ui::button_at(ui, button_rect, id, label, Look::Secondary, true);
    copy.on_hover_text(format!("Copies \u{201c}{}\u{201d}", devices::line(device))).clicked()
}
