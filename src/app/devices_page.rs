//! The page of connected devices: everything plugged into the PC or paired with it, by kind, with how each one
//! is connected and whether Windows has a problem with it. The list follows along when something is plugged in
//! or out, and the PC info tab counts the same list.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Role, Sense, Ui, WidgetInfo, pos2, vec2};

use super::{Action, App, Page, Tab, Task, finished};
use crate::devices::{self, Device, Kind};
use crate::ui::{self, Look};
use crate::applog;

/// How often the page looks whether a device was plugged in or out.
const POLL: Duration = Duration::from_secs(1);
/// How often it reads the list again anyway, as a Bluetooth device connecting changes no device Windows has.
const READ_AGAIN: Duration = Duration::from_secs(5);

pub(super) struct DevicesPage {
    /// The devices as last listed, or why Windows couldn't list them.
    pub(super) list: Option<Result<Vec<Device>, String>>,
    loading: Option<Task<Result<Vec<Device>, String>>>,
    /// What said which devices there were when they were last listed: see `devices::signal`.
    signal: u32,
    polled: Instant,
    read: Instant,
}

impl DevicesPage {
    pub(super) fn new() -> DevicesPage {
        DevicesPage { list: None, loading: None, signal: 0, polled: Instant::now(), read: Instant::now() }
    }
}

impl App {
    pub(super) fn devices_visited(&mut self) {
        self.load_devices();
    }

    pub(super) fn devices_focus_gained(&mut self) {
        if self.page == Page::Devices || (self.page == Page::Main && self.tab == Tab::Info) {
            self.load_devices();
        }
    }

    /// Lists the devices again, in the background. It takes milliseconds and touches no drive.
    pub(super) fn load_devices(&mut self) {
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
        ui::text(ui, intro, ui::regular(13.0), p.muted);
        ui.add_space(14.0);
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
                    ui::card(p).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (index, device) in list.iter().filter(|device| device.kind == *kind).enumerate() {
                            if index > 0 {
                                ui.add_space(10.0);
                            }
                            device_row(ui, device);
                        }
                    });
                }
            }
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

/// A device's name on the left, how it is connected on the right, and what is wrong with it below, if anything.
fn device_row(ui: &mut Ui, device: &Device) {
    let p = ui::palette(ui.ctx());
    let width = ui.available_width();
    let painter = ui.painter().clone();
    let connection = device.connection.map(|connection| painter.layout_no_wrap(connection.name().to_owned(), ui::regular(12.0), p.muted));
    let name_width = (width - connection.as_ref().map_or(0.0, |text| text.size().x + 16.0)).max(80.0);
    let name = painter.layout(device.name.clone(), ui::regular(13.0), p.text, name_width);
    let problem = device.problem.as_ref().map(|problem| painter.layout(problem.clone(), ui::regular(12.0), p.danger, name_width));
    let height = name.size().y + problem.as_ref().map_or(0.0, |text| 2.0 + text.size().y);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    response.widget_info(|| {
        let parts = [Some(device.name.as_str()), device.connection.map(|connection| connection.name()), device.problem.as_deref()];
        WidgetInfo::labeled(Role::Label, true, parts.into_iter().flatten().collect::<Vec<_>>().join(", "))
    });
    let name_height = name.size().y;
    painter.galley(rect.left_top(), name, p.text);
    if let Some(problem) = problem {
        painter.galley(pos2(rect.left(), rect.top() + name_height + 2.0), problem, p.danger);
    }
    if let Some(connection) = connection {
        // A pixel lower, so it shares the name's baseline.
        painter.galley(pos2(rect.right() - connection.size().x, rect.top() + 1.0), connection, p.muted);
    }
}
