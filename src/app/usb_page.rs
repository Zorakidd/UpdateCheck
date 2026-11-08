//! The page that resets a USB drive or memory card: pick it, choose how it is set up and what it is called,
//! then confirm twice that what is on it may go. Only drives that can be reset are offered, and the reset looks
//! the drive up again before it erases anything.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, CornerRadius, CursorIcon, Id, Response, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};

use super::checklist::capitalized;
use super::{Action, App, Page, Task, finished};
use crate::hardware::Disk;
use crate::ui::{self, Look};
use crate::usb::{self, FileSystem, ResetOutcome};
use crate::{applog, system};

/// How often the page looks whether a drive was plugged in or out.
const POLL: Duration = Duration::from_secs(1);
/// What has to be typed before a drive is erased.
const CONFIRM_WORD: &str = "ERASE";
/// Where a drive's text begins, right of its round button.
const LEFT: f32 = 52.0;

pub(super) struct UsbPage {
    drives: Option<Vec<Disk>>,
    loading: Option<Task<Vec<Disk>>>,
    /// What said which drives there were when they were last listed: see `drive_signal`.
    signal: (u32, u32),
    polled: Instant,
    picked: Option<Disk>,
    file_system: FileSystem,
    name: String,
    /// The two confirmations: the box ticked, and the word typed.
    copied: bool,
    typed: String,
    reset: Option<Task<Result<ResetOutcome, String>>>,
    result: Option<Result<ResetOutcome, String>>,
}

impl UsbPage {
    pub(super) fn new() -> UsbPage {
        UsbPage {
            drives: None,
            loading: None,
            signal: (0, 0),
            polled: Instant::now(),
            picked: None,
            file_system: FileSystem::ExFat,
            name: String::new(),
            copied: false,
            typed: String::new(),
            reset: None,
            result: None,
        }
    }

    fn pick(&mut self, disk: Option<Disk>) {
        self.picked = disk;
        // What was confirmed was about the drive picked before.
        self.copied = false;
        self.typed.clear();
    }
}

/// Changes when a drive is plugged in or out, cheaply enough to ask every second: the drive letters in use,
/// and the length of the list of disks Windows has, which also covers a drive without a letter, as a broken
/// stick often is. Neither touches a disk, so neither wakes a sleeping hard drive.
fn drive_signal() -> (u32, u32) {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CR_SUCCESS,
    };
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    use windows_sys::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK;
    let letters = unsafe { GetLogicalDrives() };
    let mut length = 0u32;
    let result = unsafe {
        CM_Get_Device_Interface_List_SizeW(&mut length, &GUID_DEVINTERFACE_DISK, std::ptr::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
    };
    (letters, if result == CR_SUCCESS { length } else { 0 })
}

/// Whether two listings are of the same drive, as after its files changed.
fn same_disk(a: &Disk, b: &Disk) -> bool {
    a.number == b.number && a.size == b.size && a.model == b.model && a.bus == b.bus && a.serial == b.serial
}

/// The name the drive gets: the one typed, as far as the file system takes it, or else its own or "USB".
fn label_for(name: &str, disk: &Disk, file_system: FileSystem) -> String {
    let label = usb::clean_label(name, file_system);
    if label.is_empty() { usb::default_label(disk, file_system) } else { label }
}

/// What erasing the drive will cost, in a sentence.
fn erase_text(disk: &Disk) -> String {
    let name = if disk.model.is_empty() { format!("disk {}", disk.number) } else { disk.model.clone() };
    let letters = disk.letters();
    let drive = if letters.is_empty() { name } else { format!("{name} ({})", letters.join(", ")) };
    match usb::used(disk) {
        0 => format!("Everything on {drive} will be erased. This can't be undone."),
        used => format!("Everything on {drive} will be erased: {} of files. This can't be undone.", system::size(used)),
    }
}

impl App {
    pub(super) fn usb_visited(&mut self) {
        if self.usb.reset.is_none() {
            self.usb.result = None;
        }
        self.load_drives();
    }

    pub(super) fn usb_focus_gained(&mut self) {
        if self.page == Page::Usb {
            self.load_drives();
        }
    }

    fn load_drives(&mut self) {
        if self.usb.loading.is_some() || self.usb.reset.is_some() {
            return;
        }
        self.usb.signal = drive_signal();
        self.usb.polled = Instant::now();
        self.usb.loading = Some(Task::spawn(&self.ctx, "usb drives", usb::candidates));
    }

    pub(super) fn usb_poll(&mut self) {
        let page = &self.usb;
        if self.page != Page::Usb || page.reset.is_some() || page.loading.is_some() || page.polled.elapsed() < POLL {
            return;
        }
        self.usb.polled = Instant::now();
        if drive_signal() != self.usb.signal {
            self.load_drives();
        }
    }

    pub(super) fn usb_poll_at(&self) -> Option<Instant> {
        self.usb.reset.is_none().then(|| self.usb.polled + POLL)
    }

    pub(super) fn resetting_drive(&self) -> bool {
        self.usb.reset.is_some()
    }

    pub(super) fn collect_usb(&mut self) {
        if let Some(drives) = finished(&mut self.usb.loading) {
            let drives = drives.unwrap_or_else(|| {
                applog::error("usb", "Listing the drives stopped unexpectedly.");
                Vec::new()
            });
            let page = &mut self.usb;
            let still = page.picked.as_ref().and_then(|picked| drives.iter().find(|disk| same_disk(picked, disk)));
            match still {
                // Its files may have changed, which the page says.
                Some(disk) => page.picked = Some(disk.clone()),
                None if page.picked.is_some() => page.pick(None),
                None => {}
            }
            page.drives = Some(drives);
        }
        if let Some(result) = finished(&mut self.usb.reset) {
            let result = result.unwrap_or_else(|| {
                Err("The reset stopped unexpectedly. Unplug the drive and plug it in again, then look for it in \
                     File Explorer."
                    .to_owned())
            });
            let page = &mut self.usb;
            if matches!(result, Ok(ResetOutcome::Done(_))) {
                page.name.clear();
            }
            page.pick(None);
            page.result = Some(result);
            self.load_drives();
        }
    }

    pub(super) fn usb_page(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let intro = "Erases a USB stick or memory card and sets it up again as one empty drive. This helps when it \
                     shows the wrong size, won't format, or still holds an old Windows installer.";
        ui::text(ui, intro, ui::regular(13.0), p.muted);
        ui.add_space(14.0);
        if self.usb.reset.is_some() {
            ui::card(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui::text(ui, "Resetting the drive\u{2026}", ui::bold(15.0), p.text);
                ui.add_space(2.0);
                let text = "Windows asks for administrator permission first. Don't unplug the drive until this is done.";
                ui::text(ui, text, ui::regular(12.0), p.muted);
            });
            return;
        }
        self.usb_result(ui);
        self.drive_list(ui);
        let Some(disk) = self.usb.picked.clone() else { return };
        self.drive_setup(ui, &disk);
        self.confirm_reset(ui, &disk);
    }

    fn usb_result(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let Some(result) = &self.usb.result else { return };
        let mut open = None;
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (title, text, color) = match result {
                Ok(ResetOutcome::Done(Some(letter))) => {
                    let text = format!("It is {letter} now, empty and set up again.");
                    let row = super::button_row(ui, Id::unique("open-drive"), "The drive is ready", &text, "Open in File Explorer", Look::Secondary, true);
                    if row.clicked() {
                        open = Some(format!("{letter}\\"));
                    }
                    return;
                }
                Ok(ResetOutcome::Done(None)) => (
                    "The drive was reset",
                    "Windows is still giving it a drive letter. It shows up in File Explorer in a moment.".to_owned(),
                    p.text,
                ),
                Ok(ResetOutcome::Cancelled) => (
                    "Nothing was erased",
                    "Windows didn't get administrator rights, which resetting a drive needs.".to_owned(),
                    p.text,
                ),
                Err(error) => ("Couldn't reset the drive", error.clone(), p.danger),
            };
            ui::text(ui, title, ui::bold(14.0), color);
            ui.add_space(2.0);
            ui::text(ui, text, ui::regular(12.0), p.muted);
        });
        ui.add_space(14.0);
        if let Some(path) = open {
            self.actions.push(Action::OpenUrl(path));
        }
    }

    fn drive_list(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "Drive", 0.0);
        let loading = self.usb.loading.is_some();
        let Some(drives) = self.usb.drives.clone() else {
            ui::text(ui, "Looking for drives\u{2026}", ui::regular(13.0), p.muted);
            return;
        };
        if drives.is_empty() {
            let mut refresh = false;
            ui::card(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = "No USB drive or memory card found";
                let text = "Plug one in. It shows up here by itself.";
                let label = if loading { "Looking\u{2026}" } else { "Look again" };
                refresh = super::button_row(ui, Id::unique("refresh-drives"), title, text, label, Look::Secondary, !loading).clicked();
            });
            if refresh {
                self.load_drives();
            }
            return;
        }
        let mut picked = None;
        for (index, disk) in drives.iter().enumerate() {
            if index > 0 {
                ui.add_space(8.0);
            }
            let chosen = self.usb.picked.as_ref().is_some_and(|other| same_disk(other, disk));
            if drive_row(ui, disk, chosen).clicked() && !chosen {
                picked = Some(disk.clone());
            }
        }
        if picked.is_some() {
            self.usb.pick(picked);
        }
        ui.add_space(6.0);
        let text = "Only USB drives and memory cards show up here, never the drive Windows or this app is on.";
        ui::text(ui, text, ui::regular(12.0), p.muted);
    }

    fn drive_setup(&mut self, ui: &mut Ui, disk: &Disk) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "Set up as", 18.0);
        let names: Vec<&str> = usb::FILE_SYSTEMS.iter().map(|file_system| file_system.name()).collect();
        let picked = usb::FILE_SYSTEMS.iter().position(|file_system| *file_system == self.usb.file_system).unwrap_or(0);
        if let Some(index) = ui::segmented(ui, Id::unique("file-system"), &names, picked) {
            self.usb.file_system = usb::FILE_SYSTEMS[index];
        }
        ui.add_space(6.0);
        let file_system = self.usb.file_system;
        if file_system.fits(disk.size) {
            ui::text(ui, file_system.hint(), ui::regular(12.0), p.muted);
        } else {
            let text = format!("{} works only on drives up to 32 GB. Pick exFAT instead.", file_system.name());
            ui::text(ui, text, ui::regular(12.0), p.danger);
        }

        ui::section(ui, "Name", 18.0);
        let hint = usb::default_label(disk, file_system);
        ui::text_field(ui, &mut self.usb.name, &hint);
        ui.add_space(4.0);
        let label = label_for(&self.usb.name, disk, file_system);
        ui::text(ui, format!("The drive will be called \u{201c}{label}\u{201d}."), ui::regular(12.0), p.muted);
    }

    fn confirm_reset(&mut self, ui: &mut Ui, disk: &Disk) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "Erase", 18.0);
        let fits = self.usb.file_system.fits(disk.size);
        let mut start = false;
        ui::card(p).fill(p.danger_soft).stroke(Stroke::new(1.0, p.danger)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui::text(ui, erase_text(disk), ui::bold(13.0), p.text);
            if usb::looks_like_backup(disk) {
                ui.add_space(6.0);
                let text = "This looks like an external disk rather than a USB stick. If it holds backups, they are \
                            erased too.";
                ui::text(ui, text, ui::regular(12.0), p.danger);
            }
            ui.add_space(12.0);
            let page = &mut self.usb;
            ui::check_row(ui, &mut page.copied, "I have copied anything I still need from this drive.", "");
            ui.add_space(12.0);
            let label = ui::text(ui, format!("Type {CONFIRM_WORD} to confirm."), ui::regular(12.0), p.muted);
            ui.add_space(4.0);
            // No example in the field, which would look as if the word were typed already.
            ui::text_field(ui, &mut page.typed, "").labelled_by(label.id);
            ui.add_space(12.0);
            let typed = page.typed.trim().eq_ignore_ascii_case(CONFIRM_WORD);
            let ready = page.copied && typed && fits;
            start = ui::button_with(ui, "Erase and reset drive", Look::Danger, ready).clicked();
        });
        if start && self.usb.reset.is_none() {
            let file_system = self.usb.file_system;
            let label = label_for(&self.usb.name, disk, file_system);
            let disk = disk.clone();
            self.usb.result = None;
            self.usb.reset = Some(Task::spawn(&self.ctx, "usb reset", move || usb::reset(&disk, file_system, &label)));
        }
    }
}

/// A drive to pick, as a card with a round button: picked, it takes the accent color.
fn drive_row(ui: &mut Ui, disk: &Disk, chosen: bool) -> Response {
    let p = ui::palette(ui.ctx());
    let width = ui.available_width();
    let painter = ui.painter().clone();
    let text_width = (width - LEFT - 16.0).max(80.0);
    let name = usb::describe(disk);
    let title = painter.layout(name.clone(), ui::bold(13.0), p.text, text_width);
    let used = usb::used(disk);
    let kind = capitalized(disk.kind());
    let line = if used > 0 { format!("{kind} \u{b7} {} used", system::size(used)) } else { kind };
    let line = painter.layout(line, ui::regular(12.0), p.muted, text_width);
    let height = title.size().y + 2.0 + line.size().y + 24.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), chosen, &name));
    let fill = if chosen { p.accent_soft } else { p.card };
    let edge = if chosen {
        Stroke::new(2.0, p.accent)
    } else {
        Stroke::new(1.0, if response.hovered() { p.muted } else { p.border })
    };
    painter.rect(rect, CornerRadius::same(12), fill, edge, StrokeKind::Inside);
    ui::focus_ring(ui, rect, &response, 12);
    let center = pos2(rect.left() + 26.0, rect.center().y);
    if chosen {
        painter.circle_filled(center, 10.0, p.accent);
        painter.circle_filled(center, 4.0, Color32::WHITE);
    } else {
        painter.circle_stroke(center, 9.0, Stroke::new(2.0, if response.hovered() { p.text } else { p.muted }));
    }
    let title_height = title.size().y;
    let top = rect.top() + 12.0;
    painter.galley(pos2(rect.left() + LEFT, top), title, p.text);
    painter.galley(pos2(rect.left() + LEFT, top + title_height + 2.0), line, p.muted);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Volume;

    fn stick() -> Disk {
        Disk {
            number: 2,
            model: "SanDisk Ultra".to_owned(),
            serial: "4C530001".to_owned(),
            size: 61_530_000_000,
            bus: crate::hardware::BUS_USB,
            removable_media: true,
            solid_state: None,
            volumes: vec![Volume {
                paths: vec!["E:\\".to_owned()],
                label: "Photos".to_owned(),
                file_system: "exFAT".to_owned(),
                space: Some((51_530_000_000, 61_530_000_000)),
            }],
        }
    }

    #[test]
    fn a_drive_whose_files_changed_is_still_the_same_drive() {
        let mut later = stick();
        later.volumes[0].space = Some((1_000, 61_530_000_000));
        assert!(same_disk(&stick(), &later));
        let mut other = stick();
        other.serial = "4C530002".to_owned();
        assert!(!same_disk(&stick(), &other));
    }

    #[test]
    fn the_drive_keeps_its_name_unless_one_is_typed() {
        assert_eq!(label_for("", &stick(), FileSystem::ExFat), "Photos");
        assert_eq!(label_for("  ", &stick(), FileSystem::ExFat), "Photos");
        assert_eq!(label_for("Holiday 2026", &stick(), FileSystem::Fat32), "HOLIDAY 202");
    }

    #[test]
    fn erasing_says_how_much_goes() {
        assert_eq!(erase_text(&stick()), "Everything on SanDisk Ultra (E:) will be erased: 9.31 GB of files. This can't be undone.");
        let mut empty = stick();
        empty.volumes.clear();
        empty.model.clear();
        assert_eq!(erase_text(&empty), "Everything on disk 2 will be erased. This can't be undone.");
    }
}
