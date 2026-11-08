//! The Apps tab: the apps that came with Windows or the PC and can still go, ticked off and removed for this
//! account in one go, and how to remove the few that only Settings can.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Id, Ui};

use super::checklist::Tone;
use super::{Action, App, Page, Tab, Task, finished};
use crate::apps::{self, Scan};
use crate::ui::{self, Look};
use crate::{applog, state};

/// A scan this recent is still right when the tab is opened again.
const FRESH_FOR: Duration = Duration::from_secs(5);
/// Away from the tab, coming back to the window looks again after this long: apps may have been installed or
/// removed in the meantime.
const FOCUS_RESCAN: Duration = Duration::from_secs(60);
/// The page in Settings that removes the apps the guides are about.
const INSTALLED_APPS: &str = "ms-settings:appsfeatures";

#[derive(Default)]
pub(super) struct AppsPage {
    scan: Option<Result<Scan, String>>,
    scanning: Option<Task<Result<Scan, String>>>,
    scanned_at: Option<Instant>,
    /// Whether to look again once the scan running ends, since what it finds may be out of date by then.
    again: bool,
    /// The apps ticked for removal, by package.
    selected: BTreeSet<String>,
    removal: Option<Task<Vec<apps::Outcome>>>,
    /// Which of the apps being removed is being removed now.
    removal_at: Arc<AtomicUsize>,
    removal_names: Vec<String>,
    result: Option<String>,
}

impl App {
    /// Whether the window shows what the scan finds: on the Apps tab, or on the checklist's step for it.
    pub(super) fn apps_shown(&self) -> bool {
        self.tab == Tab::Apps || self.state.steps.iter().any(|step| step.id == state::APPS_STEP)
    }

    pub(super) fn start_scan(&mut self) {
        if self.apps.removal.is_some() {
            // Removing ends with a scan of its own.
            return;
        }
        if self.apps.scanning.is_some() {
            self.apps.again = true;
            return;
        }
        self.apps.scanning = Some(Task::spawn(&self.ctx, "app scan", apps::scan));
    }

    pub(super) fn apps_visited(&mut self) {
        if self.apps.scanned_at.is_none_or(|at| at.elapsed() > FRESH_FOR) {
            self.start_scan();
        }
    }

    pub(super) fn apps_focus_gained(&mut self) {
        let on_tab = self.page == Page::Main && self.tab == Tab::Apps;
        let fresh = if on_tab { FRESH_FOR } else { FOCUS_RESCAN };
        if self.apps_shown() && self.apps.scanned_at.is_none_or(|at| at.elapsed() > fresh) {
            self.start_scan();
        }
    }

    pub(super) fn removing_apps(&self) -> bool {
        self.apps.removal.is_some()
    }

    pub(super) fn collect_apps(&mut self) {
        if let Some(result) = finished(&mut self.apps.scanning) {
            let scan = result.unwrap_or_else(|| Err("Looking for apps stopped unexpectedly.".to_owned()));
            match &scan {
                Ok(scan) => {
                    let installed: BTreeSet<&str> = scan.apps.iter().map(|app| app.package.as_str()).collect();
                    self.apps.selected.retain(|package| installed.contains(package.as_str()));
                }
                Err(error) => applog::warning("apps", format!("Couldn't look for apps: {error}")),
            }
            self.apps.scan = Some(scan);
            self.apps.scanned_at = Some(Instant::now());
            if std::mem::take(&mut self.apps.again) {
                self.start_scan();
            }
        }
        if let Some(outcomes) = finished(&mut self.apps.removal) {
            self.apps.result = Some(match outcomes {
                Some(outcomes) => apps::result_text(&outcomes),
                None => "Removing apps stopped unexpectedly. The log may say how far it got.".to_owned(),
            });
            self.apps.selected.clear();
            self.apps.removal_names.clear();
            self.start_scan();
        }
    }

    pub(super) fn start_removal(&mut self, chosen: Vec<apps::App>) {
        if self.apps.removal.is_some() || chosen.is_empty() {
            return;
        }
        self.apps.result = None;
        self.apps.removal_names = chosen.iter().map(|app| app.name.clone()).collect();
        let at = self.apps.removal_at.clone();
        at.store(0, Ordering::Relaxed);
        let ctx = self.ctx.clone();
        self.apps.removal = Some(Task::spawn(&self.ctx, "app removal", move || {
            apps::remove_all(&chosen, |index| {
                at.store(index, Ordering::Relaxed);
                ctx.request_repaint();
            })
        }));
    }

    /// The part of the checklist's line for the apps step, such as "5 apps can still be removed".
    pub(super) fn apps_meta(&self) -> Option<(String, Tone)> {
        if self.apps.removal.is_some() {
            return Some(("removing apps\u{2026}".to_owned(), Tone::Plain));
        }
        match &self.apps.scan {
            Some(Ok(scan)) => Some((scan.summary(), if scan.count() > 0 { Tone::ToDo } else { Tone::Good })),
            // The tab says what went wrong; the checklist has nothing to add that would help.
            Some(Err(_)) => None,
            None => self.apps.scanning.is_some().then(|| ("looking for apps\u{2026}".to_owned(), Tone::Plain)),
        }
    }

    /// The apps ticked for removal, in the order the tab lists them.
    fn chosen_apps(&self) -> Vec<apps::App> {
        match &self.apps.scan {
            Some(Ok(scan)) => scan.apps.iter().filter(|app| self.apps.selected.contains(&app.package)).cloned().collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn apps_tab(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let removal = self.apps.removal.as_ref().map(|_| {
            let total = self.apps.removal_names.len();
            let index = self.apps.removal_at.load(Ordering::Relaxed).min(total.saturating_sub(1));
            (self.apps.removal_names.get(index).cloned().unwrap_or_default(), index, total)
        });
        let removing = removal.is_some();
        let scanning = self.apps.scanning.is_some();
        let recommended = match &self.apps.scan {
            Some(Ok(scan)) => scan.apps.iter().filter(|app| app.recommended).map(|app| app.package.clone()).collect(),
            _ => Vec::new(),
        };
        let mut retry = false;
        let mut open_log = false;
        let mut select = None;

        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some((name, index, total)) = &removal {
                ui::text(ui, format!("Removing {name}\u{2026}"), ui::bold(15.0), p.text);
                ui.add_space(2.0);
                let text = format!("App {} of {total}. Keep the window open until it is done.", index + 1);
                ui::text(ui, text, ui::regular(12.0), p.muted);
                ui.add_space(10.0);
                ui::progress(ui, Id::unique("removal"), *index as f32 / (*total).max(1) as f32, p.accent);
                return;
            }
            match &self.apps.scan {
                None => {
                    ui::text(ui, "Looking for apps\u{2026}", ui::bold(15.0), p.text);
                }
                Some(Err(error)) => {
                    let row = super::button_row(ui, Id::unique("rescan"), "Couldn't look for apps", error, "Try again", Look::Secondary, !scanning);
                    retry = row.clicked();
                }
                Some(Ok(scan)) => {
                    // One button that selects the recommended apps, or clears what is selected.
                    let (label, everything) = if self.apps.selected.is_empty() {
                        ("Select recommended", true)
                    } else {
                        ("Clear selection", false)
                    };
                    let has_button = !everything || !recommended.is_empty();
                    let breakdown = scan.breakdown();
                    if has_button {
                        let row = super::button_row(ui, Id::unique("select"), &scan.summary(), &breakdown, label, Look::Secondary, true);
                        if row.clicked() {
                            select = Some(everything);
                        }
                    } else {
                        ui::text(ui, scan.summary(), ui::bold(14.0), p.text);
                        if !breakdown.is_empty() {
                            ui.add_space(2.0);
                            ui::text(ui, breakdown, ui::regular(12.0), p.muted);
                        }
                    }
                }
            }
            if let Some(result) = &self.apps.result {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                open_log = super::button_row(ui, Id::unique("open-log"), result, "", "Open log", Look::Secondary, true).clicked();
            }
        });

        if retry {
            self.start_scan();
        }
        if open_log {
            self.actions.push(Action::OpenUrl(apps::log_path().display().to_string()));
        }
        match select {
            Some(true) => self.apps.selected.extend(recommended),
            Some(false) => self.apps.selected.clear(),
            None => {}
        }

        let Some(Ok(scan)) = &self.apps.scan else { return };
        let (listed, guides) = (scan.apps.clone(), scan.guides.clone());
        let (recommended, others): (Vec<apps::App>, Vec<apps::App>) = listed.into_iter().partition(|app| app.recommended);
        if !recommended.is_empty() {
            ui::section(ui, "Recommended to remove", 18.0);
            self.app_list(ui, &recommended, !removing);
        }
        if !others.is_empty() {
            ui::section(ui, "Other apps", 18.0);
            ui::text(ui, "Remove these only if you don't use them.", ui::regular(12.0), p.muted);
            ui.add_space(6.0);
            self.app_list(ui, &others, !removing);
        }
        if !guides.is_empty() {
            ui::section(ui, "Remove in Settings", 18.0);
            ui::text(ui, "Only Settings can remove these. Open it, then follow the steps.", ui::regular(12.0), p.muted);
            ui.add_space(6.0);
            let mut open_settings = false;
            for (index, guide) in guides.iter().enumerate() {
                if index > 0 {
                    ui.add_space(8.0);
                }
                ui::card(p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let id = Id::unique(("guide", index));
                    if super::button_row(ui, id, &guide.name, &guide.steps, "Open Settings", Look::Secondary, true).clicked() {
                        open_settings = true;
                    }
                    if !guide.note.is_empty() {
                        ui.add_space(6.0);
                        ui::text(ui, guide.note, ui::regular(12.0), p.muted);
                    }
                });
            }
            if open_settings {
                self.actions.push(Action::OpenForStep { target: INSTALLED_APPS.to_owned(), step: state::APPS_STEP });
            }
        }
    }

    fn app_list(&mut self, ui: &mut Ui, list: &[apps::App], enabled: bool) {
        let p = ui::palette(ui.ctx());
        ui::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(enabled, |ui| {
                for (index, app) in list.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(10.0);
                    }
                    let mut on = self.apps.selected.contains(&app.package);
                    if ui::check_row(ui, &mut on, &app.name, app.note).clicked() {
                        if on {
                            self.apps.selected.insert(app.package.clone());
                        } else {
                            self.apps.selected.remove(&app.package);
                        }
                    }
                }
            });
        });
    }

    /// The Apps tab's footer, which removes what is ticked and stays in view however far the list scrolls.
    pub(super) fn apps_footer(&mut self, ui: &mut Ui) {
        let chosen = self.chosen_apps();
        let count = chosen.len();
        let removing = self.apps.removal.is_some();
        let (title, text) = if removing {
            ("Removing apps\u{2026}".to_owned(), "Keep the window open until it is done.")
        } else if count == 0 {
            ("Nothing selected".to_owned(), "Tick the apps you don't use. Only your account loses them.")
        } else {
            let title = if count == 1 { "1 app selected".to_owned() } else { format!("{count} apps selected") };
            (title, "Only your account loses them, and the Microsoft Store has them if you want one back.")
        };
        let label = if count == 0 { "Remove selected".to_owned() } else { format!("Remove selected ({count})") };
        let row = super::button_row(ui, Id::unique("remove-apps"), &title, text, &label, Look::Danger, count > 0 && !removing);
        if row.clicked() {
            self.actions.push(Action::AskRemoveApps(chosen));
        }
    }
}
