//! Settings: when the reminder comes, how the window looks, the checklist's steps, the app's own updates and
//! the tools for when Windows misbehaves.

use eframe::egui;
use egui::{ComboBox, Id, Key, Rect, RichText, Sense, Ui, pos2, vec2};

use super::{Action, App, Page, Task, finished};
use crate::state::{self, Step};
use crate::ui::{self, Look};
use crate::{system, updater};

pub(super) struct SettingsPage {
    title: String,
    detail: String,
    action: String,
    /// Whether Add was pressed without a title, which the title field then asks for.
    missing_title: bool,
    focus_title: bool,
    update: Option<Task<updater::Outcome>>,
    /// What the last look for an update found, and whether it installed one.
    update_text: Option<(String, bool)>,
    files: Option<Task<Result<bool, String>>>,
    files_text: Option<String>,
    repair: Option<Task<Result<bool, String>>>,
    repair_text: Option<String>,
}

impl SettingsPage {
    pub(super) fn new() -> SettingsPage {
        SettingsPage {
            title: String::new(),
            detail: String::new(),
            action: String::new(),
            missing_title: false,
            focus_title: false,
            update: None,
            update_text: None,
            files: None,
            files_text: None,
            repair: None,
            repair_text: None,
        }
    }

    /// Whether a tool waits for administrator rights, during which no other one starts.
    fn tool_running(&self) -> bool {
        self.files.is_some() || self.repair.is_some()
    }
}

/// What a step's Open button opens, in words for the places the default steps open.
fn opens_text(action: &str) -> String {
    let place = match action {
        "ms-settings:windowsupdate" => "Windows Update in Settings",
        "ms-settings:windowsupdate-optionalupdates" => "optional updates in Settings",
        "ms-settings:storagesense" => "Storage in Settings",
        "ms-windows-store://downloadsandupdates" => "the Library of the Microsoft Store",
        "windowsdefender://threat" => "Virus & threat protection",
        "winget upgrade --all" => return "Updates desktop apps with winget".to_owned(),
        other => other,
    };
    format!("Opens {place}")
}

/// What starting a tool in a console of its own as administrator came to.
fn started_text(result: Option<Result<bool, String>>) -> Result<String, String> {
    match result {
        Some(Ok(true)) => Ok("Started in a new window. Follow its progress there.".to_owned()),
        Some(Ok(false)) => Ok("Nothing started: Windows didn't get administrator rights.".to_owned()),
        Some(Err(error)) => Err(error),
        None => Ok("It stopped unexpectedly. The log has the details.".to_owned()),
    }
}

impl App {
    pub(super) fn collect_settings(&mut self) {
        if let Some(outcome) = finished(&mut self.settings.update) {
            self.settings.update_text = Some(match outcome {
                Some(updater::Outcome::Installed) => ("An update was installed. Restart to use it.".to_owned(), true),
                Some(updater::Outcome::UpToDate) => ("This is the newest version.".to_owned(), false),
                Some(updater::Outcome::Failed) | None => ("Couldn't look for updates. The log has the details.".to_owned(), false),
            });
        }
        if let Some(result) = finished(&mut self.settings.files) {
            match started_text(result) {
                Ok(text) => self.settings.files_text = Some(text),
                Err(error) => self.message("Couldn't start the check", error),
            }
        }
        if let Some(result) = finished(&mut self.settings.repair) {
            match started_text(result) {
                Ok(text) => self.settings.repair_text = Some(text),
                Err(error) => self.message("Couldn't start the repair", error),
            }
        }
    }

    pub(super) fn settings_page(&mut self, ui: &mut Ui) {
        self.reminder_settings(ui);
        self.appearance_settings(ui);
        self.step_settings(ui);
        self.new_step(ui);
        self.app_settings(ui);
        self.troubleshooting(ui);
    }

    fn reminder_settings(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let settings = self.state.settings.clone();
        let autostart = self.autostart;
        ui::section(ui, "Reminder", 4.0);
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let switch = ui.add_enabled_ui(system::is_release(), |ui| ui::switch(ui, autostart, "Automatic reminder")).inner;
                if switch.on_disabled_hover_text("Only the installed app can start with Windows.").clicked() {
                    self.actions.push(Action::SetAutostart(!autostart));
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui::text(ui, "Automatic reminder", ui::bold(14.0), p.text);
                    let text = if system::is_release() {
                        "Starts with Windows and opens the checklist when it is time."
                    } else {
                        "Only the installed app can start with Windows."
                    };
                    ui::text(ui, text, ui::regular(12.0), p.muted);
                });
            });
            ui.add_space(14.0);
            let mut weekday = settings.weekday % 7;
            let mut hour = settings.hour % 24;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Day").font(ui::regular(13.0)).color(p.muted));
                ComboBox::from_id_salt("weekday").width(140.0).selected_text(state::WEEKDAYS[weekday as usize]).show_ui(ui, |ui| {
                    for (index, name) in state::WEEKDAYS.iter().enumerate() {
                        ui.selectable_value(&mut weekday, index as u32, *name);
                    }
                });
                ui.add_space(16.0);
                ui.label(RichText::new("Time").font(ui::regular(13.0)).color(p.muted));
                ComboBox::from_id_salt("hour").width(90.0).selected_text(format!("{hour:02}:00")).show_ui(ui, |ui| {
                    for option in 0..24 {
                        ui.selectable_value(&mut hour, option, format!("{option:02}:00"));
                    }
                });
            });
            if (weekday, hour) != (settings.weekday % 7, settings.hour % 24) {
                self.actions.push(Action::Schedule { weekday, hour });
            }
            ui.add_space(10.0);
            let note = "The checklist starts over at this time each week. Until it is done, the reminder comes \
                        again at most once a day.";
            ui::text(ui, note, ui::regular(12.0), p.muted);
        });
    }

    fn appearance_settings(&mut self, ui: &mut Ui) {
        ui::section(ui, "Appearance", 18.0);
        let appearance = &self.state.settings.appearance;
        let picked = state::APPEARANCES.iter().position(|option| option == appearance).unwrap_or(0);
        if let Some(index) = ui::segmented(ui, Id::unique("appearance"), &state::APPEARANCES, picked) {
            self.actions.push(Action::Appearance(state::APPEARANCES[index].to_owned()));
        }
    }

    fn step_settings(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "Checklist", 18.0);
        let steps = self.state.steps.clone();
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if steps.is_empty() {
                ui::text(ui, "No steps. Add one below, or restore the default steps.", ui::regular(13.0), p.muted);
            }
            for (index, step) in steps.iter().enumerate() {
                if index > 0 {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }
                self.step_row(ui, index, steps.len(), step);
            }
        });
    }

    /// A step's title and what its Open button does, with buttons that move it up or down or remove it.
    fn step_row(&mut self, ui: &mut Ui, index: usize, count: usize, step: &Step) {
        let p = ui::palette(ui.ctx());
        let width = ui.available_width();
        let arrow = vec2(30.0, 30.0);
        let remove = ui::button_size(ui, "Remove", Look::Secondary);
        let buttons = arrow.x * 2.0 + remove.x + 12.0;
        let text_width = (width - buttons - 16.0).max(80.0);
        let painter = ui.painter().clone();
        let title = painter.layout(step.title.clone(), ui::bold(14.0), p.text, text_width);
        let what = match step.id.as_str() {
            state::GRAPHICS_STEP => "Checks your graphics drivers".to_owned(),
            state::APPS_STEP => "Opens the Apps tab".to_owned(),
            _ if step.action.trim().is_empty() => "No Open button".to_owned(),
            _ => opens_text(step.action.trim()),
        };
        let what = painter.layout(what, ui::regular(12.0), p.muted, text_width);
        let text_height = title.size().y + 2.0 + what.size().y;
        let height = text_height.max(arrow.y);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let title_height = title.size().y;
        let y = rect.center().y - text_height / 2.0;
        painter.galley(pos2(rect.left(), y), title, p.text);
        painter.galley(pos2(rect.left(), y + title_height + 2.0), what, p.muted);

        let mut x = rect.right() - remove.x;
        let middle = rect.center().y;
        let place = Rect::from_min_size(pos2(x, middle - remove.y / 2.0), remove);
        let id = ui.scope_id().with(("step", &step.id));
        if ui::button_at(ui, place, id.with("remove"), "Remove", Look::Secondary, true).clicked() {
            self.actions.push(Action::RemoveStep(index));
        }
        x -= 6.0 + arrow.x;
        let down = Rect::from_min_size(pos2(x, middle - arrow.y / 2.0), arrow);
        let response = ui::button_at(ui, down, id.with("down"), "\u{2193}", Look::Secondary, index + 1 < count);
        if response.on_hover_text("Move down").clicked() {
            self.actions.push(Action::MoveStep { from: index, to: index + 1 });
        }
        x -= 6.0 + arrow.x;
        let up = Rect::from_min_size(pos2(x, middle - arrow.y / 2.0), arrow);
        let response = ui::button_at(ui, up, id.with("up"), "\u{2191}", Look::Secondary, index > 0);
        if response.on_hover_text("Move up").clicked() {
            self.actions.push(Action::MoveStep { from: index, to: index - 1 });
        }
    }

    fn new_step(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "Add a step", 18.0);
        let mut add = false;
        let mut restore = false;
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let page = &mut self.settings;
            let title = ui::text_field(ui, &mut page.title, "Title, such as Router firmware");
            if std::mem::take(&mut page.focus_title) {
                title.request_focus();
            }
            if title.changed() {
                page.missing_title = false;
            }
            if page.missing_title {
                ui.add_space(2.0);
                ui::text(ui, "Give the step a title first.", ui::regular(12.0), p.danger);
            }
            ui.add_space(6.0);
            let detail = ui::text_field(ui, &mut page.detail, "What to check (optional)");
            ui.add_space(6.0);
            let action = ui::text_field(ui, &mut page.action, "What Open opens (optional)");
            ui.add_space(3.0);
            let hint = "A web address, a page of Settings such as ms-settings:display, or a command.";
            ui::text(ui, hint, ui::regular(12.0), p.muted);
            let enter = ui.input(|input| input.key_pressed(Key::Enter));
            add = enter && [&title, &detail, &action].iter().any(|field| field.lost_focus());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                add |= ui::button(ui, "Add step", Look::Primary).clicked();
                ui.add_space(4.0);
                restore = ui::button(ui, "Restore default steps", Look::Secondary).clicked();
            });
        });
        if add {
            let page = &mut self.settings;
            let title = page.title.trim();
            if title.is_empty() {
                page.missing_title = true;
                page.focus_title = true;
            } else {
                let step = Step::new(title, page.detail.trim(), page.action.trim());
                page.title.clear();
                page.detail.clear();
                page.action.clear();
                page.missing_title = false;
                self.actions.push(Action::AddStep(step));
            }
        }
        if restore {
            self.actions.push(Action::AskRestoreDefaults);
        }
    }

    fn app_settings(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "This app", 18.0);
        let checking = self.settings.update.is_some();
        let mut check = false;
        let mut restart = false;
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = format!("Version {}", system::VERSION);
            let (text, allowed) = if !system::is_release() {
                ("Only the installed app updates itself, not a development build.", false)
            } else if !system::self_update_allowed() {
                ("Your organization keeps this app up to date.", false)
            } else {
                ("Looks for a new version once a day, when it starts.", true)
            };
            let label = if checking { "Checking\u{2026}" } else { "Check now" };
            check = super::button_row(ui, Id::unique(("settings", "update")), &title, text, label, Look::Secondary, allowed && !checking).clicked();
            if let Some((text, installed)) = &self.settings.update_text {
                ui.add_space(10.0);
                if *installed {
                    restart = super::button_row(ui, Id::unique(("settings", "restart")), text, "", "Restart now", Look::Primary, true).clicked();
                } else {
                    ui::text(ui, text.as_str(), ui::regular(12.0), p.muted);
                }
            }
        });
        if check && self.settings.update.is_none() {
            self.settings.update_text = None;
            let data_dir = self.path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
            self.settings.update = Some(Task::spawn(&self.ctx, "self update", move || updater::self_update(&data_dir)));
        }
        if restart {
            self.actions.push(Action::Restart);
        }
    }

    fn troubleshooting(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        ui::section(ui, "When Windows misbehaves", 18.0);
        let running = self.settings.tool_running();
        let waiting = "Waiting for administrator permission\u{2026}";
        let mut check_files = false;
        let mut repair = false;
        let mut usb = false;
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let page = &self.settings;
            let text = "Finds damaged Windows files and repairs them. Takes about 15 minutes.";
            let label = if page.files.is_some() { "Waiting\u{2026}" } else { "Check files" };
            check_files = super::button_row(ui, Id::unique(("settings", "files")), "Check system files", text, label, Look::Secondary, !running).clicked();
            result_line(ui, if page.files.is_some() { Some(waiting) } else { page.files_text.as_deref() });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            let text = "Repairs Windows' own copy of its files first, then the files. Use it when checking the \
                        files isn't enough. Takes up to an hour.";
            let label = if page.repair.is_some() { "Waiting\u{2026}" } else { "Start repair" };
            repair = super::button_row(ui, Id::unique(("settings", "repair")), "Repair Windows", text, label, Look::Secondary, !running).clicked();
            result_line(ui, if page.repair.is_some() { Some(waiting) } else { page.repair_text.as_deref() });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            let text = "Erases a USB stick or memory card and sets it up again, for when it acts up.";
            usb = super::button_row(ui, Id::unique(("settings", "usb")), "Reset a USB drive", text, "Open", Look::Secondary, true).clicked();
        });
        if check_files && !self.settings.tool_running() {
            self.settings.files_text = None;
            self.settings.files = Some(Task::spawn(&self.ctx, "check files", system::check_system_files));
        }
        if repair && !self.settings.tool_running() {
            self.settings.repair_text = None;
            self.settings.repair = Some(Task::spawn(&self.ctx, "repair", system::repair_windows));
        }
        if usb {
            self.actions.push(Action::Page(Page::Usb));
        }
    }
}

/// What came of a tool, under its row.
fn result_line(ui: &mut Ui, text: Option<&str>) {
    if let Some(text) = text {
        let p = ui::palette(ui.ctx());
        ui.add_space(6.0);
        ui::text(ui, text, ui::regular(12.0), p.accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_says_what_it_opens() {
        assert_eq!(opens_text("ms-settings:windowsupdate"), "Opens Windows Update in Settings");
        assert_eq!(opens_text("winget upgrade --all"), "Updates desktop apps with winget");
        assert_eq!(opens_text("https://example.com"), "Opens https://example.com");
    }

    #[test]
    fn a_tool_says_whether_it_started() {
        assert_eq!(started_text(Some(Ok(true))).unwrap(), "Started in a new window. Follow its progress there.");
        assert_eq!(started_text(Some(Ok(false))).unwrap(), "Nothing started: Windows didn't get administrator rights.");
        assert_eq!(started_text(Some(Err("No cmd.exe".to_owned()))).unwrap_err(), "No cmd.exe");
        assert!(started_text(None).is_ok());
    }
}
