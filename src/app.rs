//! The window: the weekly checklist with its Apps and PC info tabs, Settings, the page of connected devices and
//! the page that resets a USB drive. Everything slow, such as asking Windows Update, runs on a thread of its own, so the window never
//! waits and draws only when something changed.

mod apps_tab;
mod checklist;
mod devices_page;
mod info_tab;
mod settings;
mod usb_page;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use egui::{
    Align2, Color32, CornerRadius, Frame, Id, Key, Margin, Modifiers, Rect, ScrollArea, Sense, Shadow, Stroke,
    SystemTheme, ThemePreference, Ui, UiBuilder, ViewportCommand, WindowLevel, pos2, vec2,
};

use crate::graphics::{DriverChecks, Vendor, VendorApp};
use crate::pending::{self, PendingChecks, SharedSearch, Status};
use crate::state::{self, Now, State, Step};
use crate::ui::{self, Look};
use crate::{applog, apps, system};

use apps_tab::AppsPage;
use devices_page::DevicesPage;
use info_tab::InfoPage;
use settings::SettingsPage;
use usb_page::UsbPage;

const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/window-icon.png"));

/// How often the window looks whether a new week has begun.
const TICK: Duration = Duration::from_secs(60);
/// Focus also comes back from the window's own dialogs and pop-ups, so it starts the checks again at most this
/// often.
const FOCUS_RECHECK: Duration = Duration::from_secs(3);
/// How long after an Open button the step is looked into again whenever the window gets the focus back, and
/// how long the window must have been away before it is.
const STALE_FOR: Duration = Duration::from_secs(30 * 60);
const STALE_AWAY: Duration = Duration::from_secs(5);
/// How often the window looks whether a console it opened, such as winget's, has been closed.
const CONSOLE_POLL: Duration = Duration::from_secs(1);
const UNDO_FOR: Duration = Duration::from_secs(8);
/// How long a reminder keeps the window above the others, so it isn't opened behind the window in use.
const FRONT_FOR: Duration = Duration::from_millis(1500);
/// The first start asks about the reminder shortly after the window shows, so the question comes with the
/// checklist it is about.
const ASK_AFTER: Duration = Duration::from_millis(400);
/// The keys that scroll the page, as in a browser.
const SCROLL_KEYS: [Key; 4] = [Key::PageDown, Key::PageUp, Key::Home, Key::End];

static RESTART: AtomicBool = AtomicBool::new(false);

/// Whether the window closed so that the version just installed can start in its place.
pub fn restart_requested() -> bool {
    RESTART.load(Ordering::SeqCst)
}

/// Show the window until it is closed.
pub fn open_window(state: State, path: PathBuf, reminder: bool) -> Result<(), String> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(system::TITLE)
        .with_app_id(system::APP_NAME)
        .with_inner_size([560.0, system::window_height(700.0)])
        .with_min_inner_size([460.0, 520.0]);
    match eframe::icon_data::from_png_bytes(ICON) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(error) => applog::warning("window", format!("Couldn't read the window's icon: {error}")),
    }
    let position = test_position();
    if let Some(position) = position {
        viewport = viewport.with_position(position).with_active(false);
    }
    // The integrated GPU where there are two: the window is a list of cards, and waking a gaming GPU for it
    // would cost a laptop's battery for nothing.
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.power_preference = wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower);
    let wgpu_options =
        egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup), ..Default::default() };
    let options = eframe::NativeOptions {
        viewport,
        centered: position.is_none(),
        wgpu_options,
        // Segoe UI and egui's own fonts cover the window's text; looking through every installed font for
        // more would only slow the start.
        system_font_fallback: false,
        ..Default::default()
    };
    eframe::run_native(
        system::APP_NAME,
        options,
        Box::new(move |creation| {
            ui::install(&creation.egui_ctx);
            Ok(Box::new(App::new(creation.egui_ctx.clone(), state, path, reminder)))
        }),
    )
    .map_err(|error| error.to_string())
}

/// Where a test puts the window, such as off the screen, from UPDATE_CHECK_WINDOW_POS="x,y". It then
/// doesn't take the focus either.
fn test_position() -> Option<egui::Pos2> {
    let text = std::env::var("UPDATE_CHECK_WINDOW_POS").ok()?;
    let (x, y) = text.split_once(',')?;
    Some(pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

// --- Work in the background -----------------------------------------------------------------------------

/// Work on a thread of its own, whose result the window takes in once it is done.
struct Task<T> {
    receiver: Receiver<T>,
}

/// Wakes the window when the thread that holds it ends, whether it finished or failed.
struct Wake(egui::Context);

impl Drop for Wake {
    fn drop(&mut self) {
        self.0.request_repaint();
    }
}

impl<T: Send + 'static> Task<T> {
    fn spawn(ctx: &egui::Context, name: &str, work: impl FnOnce() -> T + Send + 'static) -> Task<T> {
        let (sender, receiver) = channel();
        let wake = Wake(ctx.clone());
        let started = std::thread::Builder::new().name(name.to_owned()).spawn(move || {
            let _wake = wake;
            let _ = sender.send(work());
        });
        if let Err(error) = started {
            applog::error("window", format!("Couldn't start {name}: {error}"));
        }
        Task { receiver }
    }
}

/// The result of the task in `slot` once it has ended, which empties the slot: `Some(None)` if it ended
/// without one, as when it panicked.
fn finished<T>(slot: &mut Option<Task<T>>) -> Option<Option<T>> {
    let task = slot.as_ref()?;
    let result = match task.receiver.try_recv() {
        Ok(value) => Some(value),
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => None,
    };
    *slot = None;
    Some(result)
}

// --- The window -----------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    Checklist,
    Apps,
    Info,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Checklist, Tab::Apps, Tab::Info];
    const NAMES: [&str; 3] = ["Checklist", "Apps", "PC info"];

    fn index(self) -> usize {
        Tab::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Page {
    Main,
    Settings,
    Usb,
    Devices,
}

/// How far the page is scrolled and how far it can be, in points, as the last frame left it.
#[derive(Clone, Copy, Debug, Default)]
struct Scrolled {
    offset: f32,
    max: f32,
    page: f32,
}

impl Scrolled {
    /// How far down a key scrolls the page, or up where negative.
    fn delta(self, key: Key) -> f32 {
        // A page down keeps the last lines of the page before in sight.
        let page = (self.page - 48.0).max(48.0);
        let target = match key {
            Key::PageDown => self.offset + page,
            Key::PageUp => self.offset - page,
            Key::Home => 0.0,
            Key::End => self.max,
            _ => self.offset,
        };
        target.clamp(0.0, self.max) - self.offset
    }
}

enum Dialog {
    Message { title: String, body: String },
    AskReminder,
    RestoreDefaults,
    RemoveApps(Vec<apps::App>),
}

/// What a dialog says and the buttons it has, from left to right.
struct DialogText {
    title: String,
    body: String,
    buttons: Vec<(String, Look)>,
    /// The button the keyboard starts on: the one that changes nothing, where a dialog erases something.
    focus: usize,
}

/// A change the page asked for while it was drawn, made once it is drawn.
enum Action {
    Toggle(String),
    OpenStep(String),
    /// The vendor's app for a graphics card, or else its driver download page.
    OpenDriver { vendor: Vendor, url: String },
    Tab(Tab),
    Page(Page),
    Back,
    SetAutostart(bool),
    Schedule { weekday: u32, hour: u32 },
    Appearance(String),
    MoveStep { from: usize, to: usize },
    RemoveStep(usize),
    AddStep(Step),
    AskRestoreDefaults,
    Undo,
    Restart,
    OpenUrl(String),
    /// Open a page, such as one in Settings, where a step may get done, and look into the step again once
    /// the window is back in use.
    OpenForStep { target: String, step: &'static str },
    AskRemoveApps(Vec<apps::App>),
}

/// A step whose Open button handed over to another program, such as Settings for Windows Update. What it had
/// waiting may be installed by the time the window gets the focus back, so it is looked into again then.
struct Stale {
    id: String,
    opened: Instant,
    checked: Instant,
}

/// The checklist as it was before a step was removed or the default steps restored.
struct Undo {
    text: String,
    steps: Vec<Step>,
    /// The ticks that went with the removed step.
    ticked: Vec<String>,
    until: Instant,
}

pub struct App {
    ctx: egui::Context,
    state: State,
    path: PathBuf,
    page: Page,
    tab: Tab,
    search: Arc<SharedSearch>,
    pending: PendingChecks,
    drivers: DriverChecks,
    /// When the steps were last looked into.
    checked_at: Option<Instant>,
    storage: Option<(String, u64, u64)>,
    autostart: bool,
    /// Whether each vendor's graphics app is installed, found out once until the window gets the focus back.
    vendor_apps: BTreeMap<Vendor, Option<VendorApp>>,
    stale: Vec<Stale>,
    /// The consoles Open buttons started, such as winget's, by step.
    consoles: Vec<(String, Child)>,
    consoles_checked: Instant,
    dialogs: VecDeque<Dialog>,
    dialog_focus_pending: bool,
    actions: Vec<Action>,
    undo: Option<Undo>,
    /// The steps unticked by hand, with the week they were unticked in, so they aren't ticked off again by
    /// themselves that week.
    unticked: BTreeSet<(Option<NaiveDate>, String)>,
    save_failed_shown: bool,
    focused: bool,
    /// Whether a text field had the keyboard, or a pop-up was open, when the last frame was drawn. egui lets
    /// go of both on Escape before the next frame begins, so Escape asks these whether it was meant for them.
    typing: bool,
    popup_open: bool,
    /// How far the page was scrolled when the last frame was drawn, for the keys that scroll it.
    scrolled: Scrolled,
    next_tick: Instant,
    front_until: Option<Instant>,
    ask_at: Option<Instant>,
    apps: AppsPage,
    info: InfoPage,
    settings: SettingsPage,
    usb: UsbPage,
    devices: DevicesPage,
}

impl App {
    fn new(ctx: egui::Context, state: State, path: PathBuf, reminder: bool) -> App {
        let notify: Arc<dyn Fn() + Send + Sync> = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        let search = Arc::new(SharedSearch::new(Box::new(pending::search_windows_update)));
        let pending = PendingChecks::new(pending::standard_lookups(search.clone()), notify.clone());
        let drivers = DriverChecks::standard(notify);
        apply_appearance(&ctx, &state.settings.appearance);
        let now = Instant::now();
        let mut front_until = None;
        if reminder {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            front_until = Some(now + FRONT_FOR);
        }
        // The installed app sets itself up to start with Windows only once asked, on its first start.
        let ask_at = (system::is_release() && state.settings.autostart.is_none()).then(|| now + ASK_AFTER);
        let mut app = App {
            settings: SettingsPage::new(),
            apps: AppsPage::default(),
            info: InfoPage::default(),
            usb: UsbPage::new(),
            devices: DevicesPage::new(),
            ctx,
            state,
            path,
            page: Page::Main,
            tab: Tab::Checklist,
            search,
            pending,
            drivers,
            checked_at: None,
            storage: system::system_drive_space(),
            autostart: system::autostart_enabled(),
            vendor_apps: BTreeMap::new(),
            stale: Vec::new(),
            consoles: Vec::new(),
            consoles_checked: now,
            dialogs: VecDeque::new(),
            dialog_focus_pending: false,
            actions: Vec::new(),
            undo: None,
            unticked: BTreeSet::new(),
            save_failed_shown: false,
            focused: false,
            typing: false,
            popup_open: false,
            scrolled: Scrolled::default(),
            next_tick: now + TICK,
            front_until,
            ask_at,
        };
        app.refresh_checks(false);
        if app.apps_shown() {
            app.start_scan();
        }
        app
    }

    // --- Looking into the steps -------------------------------------------------------------------------

    /// The steps whose lookups are wanted: the checklist's, and Windows Update while the PC info tab, which
    /// says whether Windows is up to date, is open.
    fn wanted(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.state.steps.iter().map(|step| step.id.clone()).collect();
        if self.page == Page::Main && self.tab == Tab::Info {
            ids.push(pending::WINDOWS_UPDATE.to_owned());
        }
        ids
    }

    /// Start the lookups that haven't run, or failed a while ago, or all of them with `recheck`.
    fn refresh_checks(&mut self, recheck: bool) {
        self.checked_at = Some(Instant::now());
        let ids = self.wanted();
        if ids.iter().any(|id| id == state::GRAPHICS_STEP) {
            self.drivers.refresh(recheck);
        }
        self.pending.refresh(ids.iter().map(String::as_str), recheck);
    }

    /// Look into a step again, as after the user may have installed what it had waiting.
    fn recheck_step(&mut self, id: &str) {
        match id {
            state::GRAPHICS_STEP => {
                self.drivers.refresh(true);
            }
            state::APPS_STEP => self.start_scan(),
            state::STORAGE_STEP => self.storage = system::system_drive_space(),
            pending::WINDOWS_UPDATE | pending::DRIVERS => {
                // Both are counted from one search, which installing either may have changed.
                self.search.forget();
                let wanted = self.wanted();
                let ids = [pending::WINDOWS_UPDATE, pending::DRIVERS];
                self.pending.refresh(ids.into_iter().filter(|id| wanted.iter().any(|other| other == id)), true);
            }
            _ => {
                self.pending.refresh([id], true);
            }
        }
    }

    fn mark_stale(&mut self, id: &str) {
        let now = Instant::now();
        self.stale.retain(|stale| stale.id != id);
        self.stale.push(Stale { id: id.to_owned(), opened: now, checked: now });
    }

    fn focus_gained(&mut self) {
        // Coming back after freeing up space or installing a graphics app: both take microseconds to see.
        self.storage = system::system_drive_space();
        self.vendor_apps.clear();
        if self.checked_at.is_none_or(|at| at.elapsed() > FOCUS_RECHECK) {
            self.refresh_checks(false);
        }
        let now = Instant::now();
        self.stale.retain(|stale| now.duration_since(stale.opened) < STALE_FOR);
        let due: Vec<String> = self
            .stale
            .iter_mut()
            .filter(|stale| now.duration_since(stale.checked) >= STALE_AWAY)
            .map(|stale| {
                stale.checked = now;
                stale.id.clone()
            })
            .collect();
        for id in due {
            self.recheck_step(&id);
        }
        self.apps_focus_gained();
        self.usb_focus_gained();
        self.devices_focus_gained();
    }

    fn poll_consoles(&mut self) {
        if self.consoles.is_empty() || self.consoles_checked.elapsed() < CONSOLE_POLL {
            return;
        }
        self.consoles_checked = Instant::now();
        let mut closed = Vec::new();
        self.consoles.retain_mut(|(id, console)| match console.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => {
                closed.push(id.clone());
                false
            }
        });
        for id in closed {
            self.recheck_step(&id);
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if now < self.next_tick {
            return;
        }
        self.next_tick = now + TICK;
        // Space fills up while the window waits in the background, as when a game downloads.
        self.storage = system::system_drive_space();
        if self.state.roll_over(&state::local_now()) {
            self.save();
            if self.checked_at.is_some() {
                // A week on, there may be new drivers, packages and Windows updates.
                self.refresh_checks(true);
            }
        }
    }

    // --- Changes ----------------------------------------------------------------------------------------

    fn save(&mut self) {
        if let Err(error) = state::save(&self.state, &self.path) {
            applog::error("state", format!("Couldn't save the checklist to {}: {error}", self.path.display()));
            if !self.save_failed_shown {
                self.save_failed_shown = true;
                self.message(
                    "Couldn't save your checklist",
                    format!("Your changes stay in this window but may be lost when it closes.\n\n{error}"),
                );
            }
        }
    }

    fn push_dialog(&mut self, dialog: Dialog) {
        if self.dialogs.is_empty() {
            self.dialog_focus_pending = true;
        }
        self.dialogs.push_back(dialog);
    }

    fn message(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push_dialog(Dialog::Message { title: title.into(), body: body.into() });
    }

    fn set_autostart(&mut self, enabled: bool) {
        let hour = self.state.settings.hour;
        let result = if enabled { system::enable_autostart(hour) } else { system::sync_autostart(false, hour) };
        if let Err(error) = result {
            let what = if enabled { "on" } else { "off" };
            applog::error("autostart", format!("Couldn't turn the automatic reminder {what}: {error}"));
            self.message("Couldn't change the automatic reminder", error);
        }
        self.autostart = system::autostart_enabled();
        // Remembered so the app doesn't ask again on its next start.
        self.state.settings.autostart = Some(self.autostart);
        self.save();
    }

    fn show_tab(&mut self, tab: Tab) {
        self.page = Page::Main;
        self.tab = tab;
        match tab {
            Tab::Checklist => {}
            Tab::Apps => self.apps_visited(),
            Tab::Info => {
                self.info_visited();
                self.pending.refresh([pending::WINDOWS_UPDATE], false);
            }
        }
    }

    fn show_page(&mut self, page: Page) {
        self.page = page;
        match page {
            Page::Usb => self.usb_visited(),
            Page::Devices => self.devices_visited(),
            Page::Main | Page::Settings => {}
        }
    }

    fn go_back(&mut self) {
        match self.page {
            Page::Usb => self.show_page(Page::Settings),
            Page::Settings | Page::Devices => self.show_tab(self.tab),
            Page::Main => {}
        }
    }

    fn open_step(&mut self, id: &str) {
        if id == state::APPS_STEP {
            self.show_tab(Tab::Apps);
            return;
        }
        let Some(step) = self.state.steps.iter().find(|step| step.id == id) else { return };
        let action = step.action.trim().to_owned();
        if action.is_empty() {
            return;
        }
        match system::open_action(&action) {
            Ok(Some(console)) => {
                self.consoles.push((id.to_owned(), console));
                self.consoles_checked = Instant::now();
            }
            Ok(None) => self.mark_stale(id),
            Err(error) => {
                applog::warning("window", format!("Couldn't open {action}: {error}"));
                self.message(format!("Couldn't open \u{201c}{action}\u{201d}"), error);
            }
        }
    }

    fn open_driver(&mut self, vendor: Vendor, url: &str) {
        let app = self.vendor_app(vendor);
        let (result, what) = match &app {
            Some(app) => (app.open(), app.name.to_owned()),
            None => (system::open_url(url), "the download page".to_owned()),
        };
        match result {
            Ok(()) => self.mark_stale(state::GRAPHICS_STEP),
            Err(error) => {
                applog::warning("window", format!("Couldn't open {what}: {error}"));
                self.message(format!("Couldn't open {what}"), error);
            }
        }
    }

    fn vendor_app(&mut self, vendor: Vendor) -> Option<VendorApp> {
        self.vendor_apps.entry(vendor).or_insert_with(|| crate::graphics::vendor_app(vendor)).clone()
    }

    fn remember_for_undo(&mut self, text: String, ticked: Vec<String>) {
        self.undo =
            Some(Undo { text, steps: self.state.steps.clone(), ticked, until: Instant::now() + UNDO_FOR });
    }

    fn undo(&mut self) {
        let Some(undo) = self.undo.take() else { return };
        self.state.steps = undo.steps;
        self.state.done.extend(undo.ticked);
        self.save();
        self.refresh_checks(false);
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Toggle(id) => {
                let done = !self.state.done.contains(&id);
                self.state.set_done(&id, done, &state::local_now());
                if !done {
                    self.unticked.insert((self.state.cycle_start, id));
                }
                self.save();
            }
            Action::OpenStep(id) => self.open_step(&id),
            Action::OpenDriver { vendor, url } => self.open_driver(vendor, &url),
            Action::Tab(tab) => self.show_tab(tab),
            Action::Page(page) => self.show_page(page),
            Action::Back => self.go_back(),
            Action::SetAutostart(enabled) => self.set_autostart(enabled),
            Action::Schedule { weekday, hour } => {
                let settings = &mut self.state.settings;
                if (settings.weekday, settings.hour) == (weekday, hour) {
                    return;
                }
                settings.weekday = weekday;
                settings.hour = hour;
                self.state.roll_over(&state::local_now());
                if system::is_release() && self.autostart {
                    // The daily check moves to the new hour. This saves, too.
                    self.set_autostart(true);
                } else {
                    self.save();
                }
            }
            Action::Appearance(appearance) => {
                apply_appearance(&self.ctx, &appearance);
                self.state.settings.appearance = appearance;
                self.save();
            }
            Action::MoveStep { from, to } => {
                if from < self.state.steps.len() && to < self.state.steps.len() {
                    self.undo = None;
                    self.state.steps.swap(from, to);
                    self.save();
                }
            }
            Action::RemoveStep(index) => {
                if index < self.state.steps.len() {
                    let id = self.state.steps[index].id.clone();
                    let ticked = if self.state.done.contains(&id) { vec![id.clone()] } else { Vec::new() };
                    let text = format!("Removed \u{201c}{}\u{201d}", self.state.steps[index].title);
                    self.remember_for_undo(text, ticked);
                    self.state.steps.remove(index);
                    self.state.done.remove(&id);
                    self.save();
                }
            }
            Action::AddStep(step) => {
                self.undo = None;
                self.state.steps.push(step);
                self.save();
                self.refresh_checks(false);
            }
            Action::AskRestoreDefaults => self.push_dialog(Dialog::RestoreDefaults),
            Action::Undo => self.undo(),
            Action::Restart => {
                RESTART.store(true, Ordering::SeqCst);
                self.ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Action::OpenUrl(url) => {
                if let Err(error) = system::open_url(&url) {
                    applog::warning("window", format!("Couldn't open {url}: {error}"));
                    self.message(format!("Couldn't open {url}"), error);
                }
            }
            Action::OpenForStep { target, step } => match system::open_url(&target) {
                Ok(()) => self.mark_stale(step),
                Err(error) => {
                    applog::warning("window", format!("Couldn't open {target}: {error}"));
                    self.message(format!("Couldn't open {target}"), error);
                }
            },
            Action::AskRemoveApps(apps) => {
                if !apps.is_empty() {
                    self.push_dialog(Dialog::RemoveApps(apps));
                }
            }
        }
    }

    fn answer(&mut self, dialog: Dialog, answer: Option<usize>) {
        match dialog {
            Dialog::Message { .. } => {}
            // Closing the question counts as "Not now", which is remembered like any answer.
            Dialog::AskReminder => self.set_autostart(answer == Some(0)),
            Dialog::RestoreDefaults => {
                if answer == Some(0) {
                    self.remember_for_undo("Restored the default steps".to_owned(), Vec::new());
                    self.state.steps = state::default_steps();
                    self.save();
                    self.refresh_checks(false);
                }
            }
            Dialog::RemoveApps(apps) => {
                if answer == Some(0) {
                    self.start_removal(apps);
                }
            }
        }
    }

    // --- Drawing ----------------------------------------------------------------------------------------

    fn header(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let frame = Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 14 });
        egui::Panel::top("header").frame(frame).resizable(false).show_separator_line(false).show(ui, |ui| {
            match self.page {
                Page::Main => self.main_header(ui),
                Page::Settings => self.sub_header(ui, "Settings"),
                Page::Usb => self.sub_header(ui, "Reset a USB drive"),
                Page::Devices => self.sub_header(ui, "Connected devices"),
            }
        });
    }

    fn main_header(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let now = state::local_now();
        let width = ui.available_width();
        let size = ui::button_size(ui, "Settings", Look::Secondary);
        let top = ui.cursor().min;
        // Settings sits at the top right, beside the small heading and the headline.
        let button = Rect::from_min_size(pos2(top.x + width - size.x, top.y), size);
        if ui::button_at(ui, button, Id::unique("open-settings"), "Settings", Look::Secondary, true).clicked() {
            self.actions.push(Action::Page(Page::Settings));
        }
        let complete = self.state.is_complete();
        let beside = UiBuilder::new().max_rect(Rect::from_min_size(top, vec2(width - size.x - 12.0, f32::INFINITY)));
        let used = ui
            .scope_builder(beside, |ui| {
                ui.label(
                    egui::RichText::new("WEEKLY UPDATE CHECK").font(ui::bold(11.0)).color(p.accent).extra_letter_spacing(0.6),
                );
                ui::text(ui, if complete { "All caught up" } else { "Time for updates" }, ui::bold(26.0), p.text);
            })
            .response
            .rect;
        // The scope only takes the room its text needs, which may be less than the button's.
        ui.add_space((button.bottom() - used.bottom()).max(0.0));
        ui::text(ui, summary_text(&self.state, &now), ui::regular(13.0), p.muted);
        ui.add_space(14.0);
        let total = self.state.steps.len();
        let fraction = if total == 0 { 0.0 } else { self.state.done_count() as f32 / total as f32 };
        ui::progress(ui, Id::unique("progress"), fraction, if complete { p.success } else { p.accent });
        ui.add_space(16.0);
        if let Some(index) = ui::segmented(ui, Id::unique("tabs"), &Tab::NAMES, self.tab.index()) {
            self.actions.push(Action::Tab(Tab::ALL[index]));
        }
    }

    fn sub_header(&mut self, ui: &mut Ui, title: &str) {
        let p = ui::palette(ui.ctx());
        if ui::button(ui, "\u{2190} Back", Look::Secondary).clicked() {
            self.actions.push(Action::Back);
        }
        ui.add_space(10.0);
        ui::text(ui, title, ui::bold(26.0), p.text);
    }

    fn footer(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let frame = Frame::new().fill(p.card).inner_margin(Margin::symmetric(28, 14));
        let panel = egui::Panel::bottom("footer")
            .frame(frame)
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| if self.tab == Tab::Apps { self.apps_footer(ui) } else { self.footer_content(ui) });
        let rect = panel.response.rect;
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, p.border));
    }

    fn footer_content(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let painter = ui.painter().clone();
        let today = state::local_now().date_naive();
        let last = self.state.last_completed.map_or("Never".to_owned(), |when| ui::on(when.date_naive(), today));
        let last_label = painter.layout_no_wrap("Last full check".to_owned(), ui::regular(12.0), p.muted);
        let last_value = painter.layout_no_wrap(last, ui::bold(13.0), p.text);
        let width = ui.available_width();
        let right = last_label.size().x.max(last_value.size().x);
        // The switch, the gap after it, and the gap before the last check.
        let middle = (width - 52.0 - 16.0 - right).max(80.0);
        let title = painter.layout("Automatic reminder".to_owned(), ui::bold(13.0), p.text, middle);
        let schedule = painter.layout(schedule_text(&self.state.settings, self.autostart), ui::regular(12.0), p.muted, middle);
        let middle_height = title.size().y + 1.0 + schedule.size().y;
        let right_height = last_label.size().y + 1.0 + last_value.size().y;
        let height = middle_height.max(right_height).max(22.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());

        let switch = Rect::from_min_size(pos2(rect.left(), rect.center().y - 11.0), vec2(40.0, 22.0));
        let mut place = UiBuilder::new().max_rect(switch);
        if !system::is_release() {
            place = place.disabled();
        }
        let response = ui.scope_builder(place, |ui| ui::switch(ui, self.autostart, "Automatic reminder")).inner;
        let response = response.on_disabled_hover_text("Only the installed app can start with Windows.");
        if response.clicked() {
            self.actions.push(Action::SetAutostart(!self.autostart));
        }

        let mut y = rect.center().y - middle_height / 2.0;
        let title_height = title.size().y;
        painter.galley(pos2(rect.left() + 52.0, y), title, p.text);
        y += title_height + 1.0;
        painter.galley(pos2(rect.left() + 52.0, y), schedule, p.muted);

        let mut y = rect.center().y - right_height / 2.0;
        let label_height = last_label.size().y;
        painter.galley(pos2(rect.right() - last_label.size().x, y), last_label, p.muted);
        y += label_height + 1.0;
        painter.galley(pos2(rect.right() - last_value.size().x, y), last_value, p.text);
    }

    fn page_area(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        // Page Up and Page Down, Home and End scroll the page, unless a text field, a dialog or a pop-up has them.
        let busy = !self.dialogs.is_empty() || self.typing || self.popup_open;
        let key = if busy { None } else { ui.input_mut(|input| SCROLL_KEYS.into_iter().find(|key| input.consume_key(Modifiers::NONE, *key))) };
        let delta = key.map(|key| self.scrolled.delta(key));
        egui::CentralPanel::no_frame().frame(Frame::new().fill(p.bg)).show(ui, |ui| {
            let scroll = ScrollArea::vertical().id_salt(("scroll", self.page, self.tab)).auto_shrink(false);
            let output = scroll.show(ui, |ui| {
                if let Some(delta) = delta {
                    ui.scroll_with_delta(vec2(0.0, -delta));
                }
                // Room above for the focus ring of the first card, which the scrolled area would cut off.
                Frame::new().inner_margin(Margin { left: 28, right: 28, top: 4, bottom: 16 }).show(ui, |ui| {
                    match (self.page, self.tab) {
                        (Page::Main, Tab::Checklist) => self.checklist(ui),
                        (Page::Main, Tab::Apps) => self.apps_tab(ui),
                        (Page::Main, Tab::Info) => self.info_tab(ui),
                        (Page::Settings, _) => self.settings_page(ui),
                        (Page::Usb, _) => self.usb_page(ui),
                        (Page::Devices, _) => self.devices_page(ui),
                    }
                });
            });
            let page = output.inner_rect.height();
            self.scrolled = Scrolled { offset: output.state.clamped_offset().y, max: (output.content_size.y - page).max(0.0), page };
        });
    }

    fn dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialogs.front() else { return };
        let p = ui::palette(ctx);
        let text = dialog_text(dialog, &self.state.settings);
        let focus = std::mem::take(&mut self.dialog_focus_pending);
        let shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(70) };
        let frame = ui::card(p).inner_margin(Margin::same(24)).shadow(shadow);
        let modal = egui::Modal::new(Id::unique("dialog")).frame(frame).show(ctx, |ui| {
            ui.set_width(380.0);
            ui::text(ui, &text.title, ui::bold(16.0), p.text);
            ui.add_space(8.0);
            ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                ui::text(ui, &text.body, ui::regular(13.0), p.text);
            });
            ui.add_space(20.0);
            let mut answer = None;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // From the right, so the first button ends up on the left, as Windows places it.
                for (index, (label, look)) in text.buttons.iter().enumerate().rev() {
                    let response = ui::button(ui, label, *look);
                    if focus && index == text.focus {
                        response.request_focus();
                    }
                    if response.clicked() {
                        answer = Some(index);
                    }
                }
            });
            answer
        });
        let answer = match modal.inner {
            Some(index) => Some(index),
            None if modal.should_close() => None,
            None => return,
        };
        if let Some(dialog) = self.dialogs.pop_front() {
            self.dialog_focus_pending = !self.dialogs.is_empty();
            self.answer(dialog, answer);
        }
    }

    fn undo_toast(&mut self, ctx: &egui::Context) {
        let Some(undo) = &self.undo else { return };
        let p = ui::palette(ctx);
        let text = format!("{}.", undo.text);
        // Above the footer on the checklist, and near the bottom elsewhere.
        let offset = if self.page == Page::Main { -96.0 } else { -24.0 };
        egui::Area::new(Id::unique("undo")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, offset)).order(egui::Order::Foreground).show(
            ctx,
            |ui| {
                Frame::new().fill(p.text).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(16, 10)).show(
                    ui,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(text).font(ui::regular(13.0)).color(p.bg));
                            ui.add_space(12.0);
                            if ui::button(ui, "Undo", Look::Secondary).on_hover_text("Ctrl+Z").clicked() {
                                self.actions.push(Action::Undo);
                            }
                        });
                    },
                );
            },
        );
    }

    fn schedule_repaint(&self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut next = self.next_tick;
        let mut consider = |at: Option<Instant>| {
            if let Some(at) = at {
                next = next.min(at);
            }
        };
        consider(self.front_until);
        consider(self.ask_at);
        consider(self.undo.as_ref().map(|undo| undo.until));
        consider(self.info.copied_until);
        consider(self.devices.copied.map(|(_, until)| until));
        if !self.consoles.is_empty() {
            consider(Some(self.consoles_checked + CONSOLE_POLL));
        }
        if self.page == Page::Usb {
            consider(self.usb_poll_at());
        }
        if self.page == Page::Devices {
            consider(Some(self.devices_poll_at()));
        }
        ctx.request_repaint_after(next.saturating_duration_since(now));
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pending.collect();
        self.tick_steps_with_nothing_waiting();
        self.drivers.collect();
        self.collect_apps();
        self.collect_info();
        self.collect_settings();
        self.collect_usb();
        self.collect_devices();
        self.poll_consoles();
        self.tick();
        self.usb_poll();
        self.devices_poll();

        let focused = ctx.input(|input| input.focused);
        if focused != self.focused {
            self.focused = focused;
            if focused {
                self.focus_gained();
            }
        }

        let busy = !self.dialogs.is_empty() || self.typing || self.popup_open;
        if !busy && self.page != Page::Main && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.go_back();
        }
        if self.undo.is_some() && !busy && ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            let still = if self.removing_apps() {
                Some(("Still removing apps", "Wait a moment until it is done, so the log can record what was removed."))
            } else if self.resetting_drive() {
                Some(("Still resetting the drive", "Wait until it is done, and don't unplug the drive until then."))
            } else {
                None
            };
            if let Some((title, body)) = still {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                let shown = self.dialogs.iter().any(|dialog| matches!(dialog, Dialog::Message { title: shown, .. } if shown == title));
                if !shown {
                    self.message(title, body);
                }
            }
        }

        let now = Instant::now();
        if self.front_until.is_some_and(|until| now >= until) {
            self.front_until = None;
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        }
        if self.ask_at.is_some_and(|at| now >= at) {
            self.ask_at = None;
            self.push_dialog(Dialog::AskReminder);
        }
        if self.undo.as_ref().is_some_and(|undo| now >= undo.until) {
            self.undo = None;
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.header(ui);
        if self.page == Page::Main {
            self.footer(ui);
        }
        self.page_area(ui);
        self.dialog(&ctx);
        self.undo_toast(&ctx);
        for action in std::mem::take(&mut self.actions) {
            self.apply(action);
        }
        self.typing = ctx.egui_wants_keyboard_input();
        self.popup_open = ctx.any_popup_open();
        self.schedule_repaint(&ctx);
    }
}

// --- Words ----------------------------------------------------------------------------------------------

fn apply_appearance(ctx: &egui::Context, appearance: &str) {
    let (preference, title_bar) = match appearance {
        "Light" => (ThemePreference::Light, SystemTheme::Light),
        "Dark" => (ThemePreference::Dark, SystemTheme::Dark),
        _ => (ThemePreference::System, SystemTheme::SystemDefault),
    };
    ctx.set_theme(preference);
    ctx.send_viewport_cmd(ViewportCommand::SetTheme(title_bar));
}

/// The line under the headline: how far the week has got, or when the next one begins.
fn summary_text(state: &State, now: &Now) -> String {
    let streak = state.streak(now);
    // One finished week isn't a run worth mentioning yet.
    let run = if streak >= 2 { format!("{streak} weeks running") } else { String::new() };
    if state.is_complete() {
        let nice = if run.is_empty() { "Nice work.".to_owned() } else { format!("Nice work \u{2014} {run}.") };
        let next = ui::when(state.next_reminder(now), state.settings.hour);
        return format!("{nice} Next check is {next}.");
    }
    let week = state.cycle_start.unwrap_or_else(|| state.current_cycle_start(now));
    let mut text = format!("{} of {} steps done \u{b7} week of {}", state.done_count(), state.steps.len(), ui::day(week));
    if !run.is_empty() {
        text.push_str(&format!(" \u{b7} {run}"));
    }
    text
}

/// When the reminder comes, under the switch in the footer.
fn schedule_text(settings: &state::Settings, on: bool) -> String {
    let weekday = state::WEEKDAYS[settings.weekday as usize % 7];
    let schedule = format!("{weekday}s at {:02}:00", settings.hour % 24);
    if on { format!("{schedule}, or at sign-in if missed") } else { format!("Off \u{b7} turn on to be reminded {schedule}") }
}

/// What the PC info tab says about Windows Update: whether anything is waiting, and whether a restart is.
pub fn windows_update_text(status: Status, restart: bool) -> String {
    let mut text = match status {
        Status::Checking => "Asking Windows Update\u{2026}".to_owned(),
        // The restart is said below, from `restart`, which the tab reads afresh on each visit.
        Status::NothingPending | Status::RestartPending => "Up to date".to_owned(),
        Status::Pending(count) => pending::describe(pending::WINDOWS_UPDATE, count),
        Status::CheckFailed => "Couldn't ask Windows Update".to_owned(),
    };
    if restart {
        text.push_str("\nRestart to finish installing updates");
    }
    text
}

fn dialog_text(dialog: &Dialog, settings: &state::Settings) -> DialogText {
    let ok = || vec![("OK".to_owned(), Look::Primary)];
    match dialog {
        Dialog::Message { title, body } => DialogText { title: title.clone(), body: body.clone(), buttons: ok(), focus: 0 },
        Dialog::AskReminder => {
            let weekday = state::WEEKDAYS[settings.weekday as usize % 7];
            let when = format!("{weekday}s at {:02}:00", settings.hour % 24);
            DialogText {
                title: "Remind you to work through the checklist every week?".to_owned(),
                body: format!(
                    "Update Check will then start with Windows and open on {when}, or when you next sign in if \
                     the PC was off. Until the week's steps are done, it reminds you at most once a day. No \
                     administrator rights are needed.\n\nThe switch at the bottom of the window turns this on \
                     or off at any time."
                ),
                buttons: vec![("Turn on reminder".to_owned(), Look::Primary), ("Not now".to_owned(), Look::Secondary)],
                focus: 0,
            }
        }
        Dialog::RestoreDefaults => DialogText {
            title: "Replace your checklist with the default steps?".to_owned(),
            body: "Steps you added yourself are removed. Right afterwards, Undo brings them back.".to_owned(),
            buttons: vec![("Replace checklist".to_owned(), Look::Danger), ("Cancel".to_owned(), Look::Secondary)],
            focus: 1,
        },
        Dialog::RemoveApps(apps) => {
            let count = apps.len();
            let (title, button) = match apps.as_slice() {
                [app] if app.turns_off() => (format!("Turn off {}?", app.name), "Turn off".to_owned()),
                [app] => (format!("Remove {}?", app.name), "Remove".to_owned()),
                _ => (format!("Remove {count} apps?"), format!("Remove {count} apps")),
            };
            let names: Vec<String> = apps
                .iter()
                .map(|app| match app.turns_off() && count > 1 {
                    true => format!("\u{2022} {} (turned off)", app.name),
                    false => format!("\u{2022} {}", app.name),
                })
                .collect();
            // Resume is the one app turned off instead.
            let removed = apps.iter().filter(|app| !app.turns_off()).count();
            let mut notes = Vec::new();
            if removed > 0 {
                let again = match (removed, removed == count) {
                    (1, true) => "it",
                    (_, true) => "them",
                    (1, false) => "the removed app",
                    _ => "the removed apps",
                };
                notes.push(format!("You can install {again} again from the Microsoft Store at any time."));
            }
            if removed < count {
                notes.push("Resume is turned off for everyone on this PC. The end of the Apps tab turns it on again.".to_owned());
            }
            DialogText {
                title,
                body: format!("{}\n\n{}", names.join("\n"), notes.join(" ")),
                buttons: vec![(button, Look::Danger), ("Cancel".to_owned(), Look::Secondary)],
                focus: 1,
            }
        }
    }
}

/// A title and a line under it on the left, and a button on the right, centered on each other. Returns the
/// button's response.
fn button_row(ui: &mut Ui, id: Id, title: &str, text: &str, button: &str, look: Look, enabled: bool) -> egui::Response {
    let mut responses = buttons_row(ui, id, title, text, &[(button, look)], enabled);
    responses.remove(0)
}

/// Like `button_row`, with several buttons side by side on the right, in the order given. Returns their
/// responses in that order.
fn buttons_row(ui: &mut Ui, id: Id, title: &str, text: &str, buttons: &[(&str, Look)], enabled: bool) -> Vec<egui::Response> {
    const GAP: f32 = 8.0;
    let p = ui::palette(ui.ctx());
    let width = ui.available_width();
    let sizes: Vec<_> = buttons.iter().map(|&(label, look)| ui::button_size(ui, label, look)).collect();
    let buttons_width = sizes.iter().map(|size| size.x).sum::<f32>() + GAP * sizes.len().saturating_sub(1) as f32;
    let buttons_height = sizes.iter().map(|size| size.y).fold(0.0, f32::max);
    let text_width = (width - buttons_width - 16.0).max(80.0);
    let painter = ui.painter().clone();
    let title = painter.layout(title.to_owned(), ui::bold(14.0), p.text, text_width);
    let text = (!text.is_empty()).then(|| painter.layout(text.to_owned(), ui::regular(12.0), p.muted, text_width));
    let text_height = title.size().y + text.as_ref().map_or(0.0, |text| 2.0 + text.size().y);
    let height = text_height.max(buttons_height);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut y = rect.center().y - text_height / 2.0;
    let title_height = title.size().y;
    painter.galley(pos2(rect.left(), y), title, p.text);
    y += title_height + 2.0;
    if let Some(text) = text {
        painter.galley(pos2(rect.left(), y), text, p.muted);
    }
    let mut left = rect.right() - buttons_width;
    let mut responses = Vec::with_capacity(buttons.len());
    for (index, (&(label, look), size)) in buttons.iter().zip(sizes).enumerate() {
        let place = Rect::from_min_size(pos2(left, rect.center().y - size.y / 2.0), size);
        // The first button keeps the row's own id, so a row with one button behaves as it always has.
        let button_id = if index == 0 { id } else { id.with(index) };
        responses.push(ui::button_at(ui, place, button_id, label, look, enabled));
        left += size.x + GAP;
    }
    responses
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate, TimeZone};

    fn at(year: i32, month: u32, day: u32, hour: u32) -> Now {
        FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn checklist() -> State {
        let mut state = State::with_steps(
            vec![Step::new("One", "", ""), Step::new("Two", "", "")],
            state::Settings { weekday: 0, hour: 9, appearance: "System".into(), autostart: None },
        );
        state.roll_over(&at(2026, 11, 4, 12));
        state
    }

    #[test]
    fn the_summary_counts_the_week_s_steps() {
        let mut state = checklist();
        let now = at(2026, 11, 4, 12);
        assert_eq!(summary_text(&state, &now), "0 of 2 steps done \u{b7} week of Nov 2");
        let id = state.steps[0].id.clone();
        state.set_done(&id, true, &now);
        assert_eq!(summary_text(&state, &now), "1 of 2 steps done \u{b7} week of Nov 2");
    }

    #[test]
    fn a_finished_week_says_when_the_next_begins() {
        let mut state = checklist();
        let now = at(2026, 11, 4, 12);
        for id in state.steps.iter().map(|step| step.id.clone()).collect::<Vec<_>>() {
            state.set_done(&id, true, &now);
        }
        assert_eq!(summary_text(&state, &now), "Nice work. Next check is Mon, Nov 9 at 09:00.");
        state.completed_weeks.insert(0, NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
        assert_eq!(summary_text(&state, &now), "Nice work \u{2014} 2 weeks running. Next check is Mon, Nov 9 at 09:00.");
    }

    #[test]
    fn the_footer_says_when_the_reminder_comes() {
        let settings = state::Settings { weekday: 2, hour: 7, appearance: "System".into(), autostart: Some(true) };
        assert_eq!(schedule_text(&settings, true), "Wednesdays at 07:00, or at sign-in if missed");
        assert_eq!(schedule_text(&settings, false), "Off \u{b7} turn on to be reminded Wednesdays at 07:00");
    }

    #[test]
    fn windows_update_says_what_is_waiting_and_whether_to_restart() {
        assert_eq!(windows_update_text(Status::NothingPending, false), "Up to date");
        assert_eq!(windows_update_text(Status::RestartPending, true), "Up to date\nRestart to finish installing updates");
        assert_eq!(windows_update_text(Status::Pending(1), false), "1 update waiting");
        assert_eq!(
            windows_update_text(Status::Pending(3), true),
            "3 updates waiting\nRestart to finish installing updates"
        );
        assert_eq!(windows_update_text(Status::CheckFailed, false), "Couldn't ask Windows Update");
    }

    #[test]
    fn the_keys_scroll_by_pages_and_stop_at_the_ends() {
        let scrolled = Scrolled { offset: 100.0, max: 1000.0, page: 548.0 };
        assert_eq!(scrolled.delta(Key::PageDown), 500.0);
        assert_eq!(scrolled.delta(Key::PageUp), -100.0);
        assert_eq!(scrolled.delta(Key::Home), -100.0);
        assert_eq!(scrolled.delta(Key::End), 900.0);
        assert_eq!(Scrolled { offset: 900.0, ..scrolled }.delta(Key::PageDown), 100.0);
        assert_eq!(Scrolled::default().delta(Key::PageDown), 0.0);
    }

    #[test]
    fn removing_apps_starts_on_cancel() {
        let settings = state::Settings::default();
        let text = dialog_text(&Dialog::RestoreDefaults, &settings);
        assert_eq!(text.buttons[text.focus].0, "Cancel");
        let text = dialog_text(&Dialog::RemoveApps(Vec::new()), &settings);
        assert_eq!(text.buttons[text.focus].0, "Cancel");
    }
}
