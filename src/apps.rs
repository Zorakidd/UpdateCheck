//! Preinstalled apps: the ones that come with Windows or a new PC and that hardly anyone uses. Most can be
//! removed for this account without administrator rights. A few, such as Microsoft Edge, only Settings can
//! remove, so for those the app explains where. Resume is part of Windows, so it is turned off instead.
//!
//! Only apps on the list below are ever offered. Everything else, including what Windows itself needs and the
//! everyday apps such as Calculator, Photos or Notepad, is left alone.

use std::collections::BTreeSet;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::time::Duration;

use chrono::NaiveDateTime;
use windows::ApplicationModel::PackageSignatureKind;
use windows::Management::Deployment::{PackageManager, PackageTypes};
use windows::core::{HRESULT, HSTRING};

use crate::applog;
use crate::system::{self, Hive};

/// An app on the list, by the name of its package.
pub struct Known {
    /// The package's name, or the start of it where it ends in a dot, for a publisher's games.
    pub package: &'static str,
    /// The app's name, or empty to use the name the package gives itself.
    pub name: &'static str,
    /// Why someone might keep it; empty if hardly anyone does.
    pub note: &'static str,
    /// Whether "Select recommended" picks it: apps Microsoft has retired, trials, games and promotions.
    pub recommended: bool,
}

const fn app(package: &'static str, name: &'static str, note: &'static str, recommended: bool) -> Known {
    Known { package, name, note, recommended }
}

const RETIRED: &str = "Retired by Microsoft.";

pub const CATALOG: &[Known] = &[
    // Retired, or almost never used.
    app("Microsoft.3DBuilder", "3D Builder", "", true),
    app("Microsoft.Microsoft3DViewer", "3D Viewer", "", true),
    app("Microsoft.Print3D", "Print 3D", "", true),
    app("Microsoft.MSPaint", "Paint 3D", "Not the normal Paint, which stays.", true),
    app("Microsoft.549981C3F5F10", "Cortana", RETIRED, true),
    app("Microsoft.BingNews", "Microsoft News", "", true),
    app("Microsoft.News", "Microsoft News", "", true),
    app("Microsoft.BingFinance", "MSN Money", "", true),
    app("Microsoft.BingSports", "MSN Sports", "", true),
    app("Microsoft.BingTravel", "MSN Travel", "", true),
    app("Microsoft.BingFoodAndDrink", "MSN Food & Drink", "", true),
    app("Microsoft.BingHealthAndFitness", "MSN Health & Fitness", "", true),
    app("Microsoft.Getstarted", "Tips", "", true),
    app("Microsoft.MicrosoftOfficeHub", "Microsoft 365 (Office)", "Only a launcher. Installed Office apps stay.", true),
    app("Microsoft.MicrosoftSolitaireCollection", "Solitaire Collection", "", true),
    app("Microsoft.WindowsFeedbackHub", "Feedback Hub", "", true),
    app("Microsoft.WindowsMaps", "Maps", RETIRED, true),
    app("Microsoft.WindowsCommunicationsApps", "Mail and Calendar", "Retired. The new Outlook replaced it.", true),
    app("Microsoft.People", "People", RETIRED, true),
    app("Microsoft.Messaging", "Messaging", RETIRED, true),
    app("Microsoft.SkypeApp", "Skype", RETIRED, true),
    app("MicrosoftTeams", "Teams (free)", RETIRED, true),
    app("Microsoft.MixedReality.Portal", "Mixed Reality Portal", "Only for VR headsets.", true),
    app("Microsoft.Windows.DevHome", "Dev Home", RETIRED, true),
    app("Microsoft.Wallet", "Microsoft Pay", RETIRED, true),
    app("Microsoft.OneConnect", "Mobile Plans", "", true),
    app("Microsoft.Office.Sway", "Sway", "", true),
    app("Microsoft.NetworkSpeedTest", "Network Speed Test", "", true),
    app("Microsoft.XboxApp", "Xbox Console Companion", "Retired. The Xbox app replaced it.", true),
    // Trials, promotions and games that come with a new PC.
    app("7EE7776C.LinkedInforWindows", "LinkedIn", "", true),
    app("king.com.", "", "", true),
    app("A278AB0D.", "", "", true),
    app("Nordcurrent.CookingFever", "Cooking Fever", "", true),
    app("flaregamesGmbH.RoyalRevolt2", "Royal Revolt 2", "", true),
    app("Playtika.CaesarsSlotsFreeCasino", "Caesars Slots", "", true),
    app("ActiproSoftwareLLC.562882FEEB491", "Code Writer", "", true),
    app("46928bounde.EclipseManager", "Eclipse Manager", "", true),
    app("PricelinePartnerNetwork.Booking.comBigsavingsonhot", "Booking.com", "", true),
    // Useful to some: offered, but never picked for you.
    app("Microsoft.BingWeather", "Weather", "", false),
    app("Microsoft.BingSearch", "Bing Search", "Gives Windows Search its web results.", false),
    app("Microsoft.GetHelp", "Get Help", "Some troubleshooting links open it.", false),
    app("Clipchamp.Clipchamp", "Clipchamp", "A video editor.", false),
    app("Microsoft.Copilot", "Copilot", "Microsoft's AI assistant.", false),
    app("Microsoft.OutlookForWindows", "Outlook (new)", "Keep it if you read your mail with it.", false),
    app("Microsoft.Todos", "Microsoft To Do", "Keep it if you keep lists in it.", false),
    app("Microsoft.MicrosoftStickyNotes", "Sticky Notes", "Keep it if you keep notes in it.", false),
    app("Microsoft.Office.OneNote", "OneNote for Windows 10", "Retired. Your notes stay in OneDrive.", false),
    app("Microsoft.WindowsSoundRecorder", "Sound Recorder", "", false),
    app("Microsoft.WindowsAlarms", "Clock", "Alarms, timers and focus sessions.", false),
    app("Microsoft.YourPhone", "Phone Link", "Connects your phone to the PC.", false),
    app("MicrosoftCorporationII.QuickAssist", "Quick Assist", "Lets someone you trust help you remotely.", false),
    app("MSTeams", "Microsoft Teams", "Keep it if you use Teams for work or school.", false),
    app("Microsoft.PowerAutomateDesktop", "Power Automate", "Keep it if you have automated tasks.", false),
    app("SpotifyAB.SpotifyMusic", "Spotify", "Keep it if you listen with it.", false),
    app("MicrosoftWindows.Client.WebExperience", "Widgets", "The news and weather board on the taskbar.", false),
    app("Microsoft.ZuneVideo", "Films & TV", "Media Player plays videos too.", false),
    app("Microsoft.GamingApp", "Xbox", "Needed for PC Game Pass and some games.", false),
    app("Microsoft.XboxGamingOverlay", "Game Bar", "Records clips and shows performance in games.", false),
    app("Microsoft.XboxSpeechToTextOverlay", "Xbox speech to text", "Captions for voice chat in games.", false),
    app("Microsoft.Edge.GameAssist", "Edge Game Assist", "A browser on top of games.", false),
];

/// The list's entry for a package, by the package's name.
pub fn known(package: &str) -> Option<&'static Known> {
    CATALOG.iter().find(|known| {
        if known.package.ends_with('.') {
            package.len() > known.package.len() && package.starts_with(known.package)
        } else {
            package.eq_ignore_ascii_case(known.package)
        }
    })
}

/// An app on the list that is installed for this account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub note: &'static str,
    pub recommended: bool,
    /// The package's name, which the log records.
    pub package: String,
    removal: Removal,
}

/// How an app goes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Removal {
    /// Its packages are removed: one per version or processor type the app is installed in.
    Packages(Vec<String>),
    /// Resume is part of a package Windows needs, so it is turned off instead.
    TurnOffResume,
}

impl App {
    /// Whether the app is turned off rather than removed.
    pub fn turns_off(&self) -> bool {
        self.removal == Removal::TurnOffResume
    }
}

/// An app only Settings can remove, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guide {
    pub name: String,
    pub steps: String,
    pub note: &'static str,
}

/// What is installed that could still go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub apps: Vec<App>,
    pub guides: Vec<Guide>,
    /// Whether Resume is turned off, so the tab can offer to turn it on again.
    pub resume_off: bool,
}

impl Scan {
    pub fn count(&self) -> usize {
        self.apps.len() + self.guides.len()
    }

    pub fn recommended(&self) -> usize {
        self.apps.iter().filter(|app| app.recommended).count()
    }

    /// The line the checklist and the Apps tab show, such as "5 apps can still be removed".
    pub fn summary(&self) -> String {
        match self.count() {
            0 => "Nothing left to remove".to_owned(),
            1 => "1 app can still be removed".to_owned(),
            count => format!("{count} apps can still be removed"),
        }
    }

    /// Where they can be removed, such as "3 recommended · 2 more here · 1 in Settings". Empty when all of them
    /// are optional apps on this tab, as the list says as much.
    pub fn breakdown(&self) -> String {
        let mut parts = Vec::new();
        let recommended = self.recommended();
        if recommended > 0 {
            parts.push(format!("{recommended} recommended"));
        }
        let others = self.apps.len() - recommended;
        if others > 0 && (recommended > 0 || !self.guides.is_empty()) {
            parts.push(format!("{others} {}", if recommended > 0 { "more here" } else { "here" }));
        }
        if !self.guides.is_empty() {
            parts.push(format!("{} in Settings", self.guides.len()));
        }
        parts.join(" · ")
    }

    /// What the scan found, without the apps set aside with Ignore. `ignored` holds their packages, and the
    /// names of those only Settings removes.
    pub fn without(&self, ignored: &BTreeSet<String>) -> Scan {
        Scan {
            apps: self.apps.iter().filter(|app| !ignored.contains(&app.package)).cloned().collect(),
            guides: self.guides.iter().filter(|guide| !ignored.contains(&guide.name)).cloned().collect(),
            resume_off: self.resume_off,
        }
    }

    /// The apps set aside with Ignore that are still installed, as their key in `ignored` and their name, in
    /// the order the tab lists them.
    pub fn ignored(&self, ignored: &BTreeSet<String>) -> Vec<(String, String)> {
        let apps = self.apps.iter().filter(|app| ignored.contains(&app.package)).map(|app| (app.package.clone(), app.name.clone()));
        let guides = self.guides.iter().filter(|guide| ignored.contains(&guide.name)).map(|guide| (guide.name.clone(), guide.name.clone()));
        apps.chain(guides).collect()
    }
}

/// Look for the apps on the list that are installed for this account, and for the ones only Settings removes.
/// Takes a second or so, so it belongs on a background thread.
pub fn scan() -> Result<Scan, String> {
    let manager = PackageManager::new().map_err(|error| format!("Windows couldn't list the apps: {}", error.message()))?;
    // Only apps: frameworks, language resources and bundles aren't anything one removes on their own.
    let packages = manager
        .FindPackagesByUserSecurityIdWithPackageTypes(&HSTRING::new(), PackageTypes::Main)
        .map_err(|error| format!("Windows couldn't list the apps: {}", error.message()))?;
    let mut apps: Vec<App> = Vec::new();
    for package in packages {
        let Ok(id) = package.Id() else { continue };
        let Ok(name) = id.Name().map(|name| name.to_string()) else { continue };
        let Some(known) = known(&name) else { continue };
        // Parts of Windows are signed by Windows itself and can't be removed for one account.
        if package.SignatureKind().is_ok_and(|kind| kind == PackageSignatureKind::System) {
            continue;
        }
        let Ok(full_name) = id.FullName().map(|name| name.to_string()) else { continue };
        if let Some(app) = apps.iter_mut().find(|app| app.package == name) {
            if let Removal::Packages(full_names) = &mut app.removal {
                full_names.push(full_name);
            }
            continue;
        }
        let display = if known.name.is_empty() {
            let own = package.DisplayName().map(|name| name.to_string()).unwrap_or_default();
            if own.is_empty() || own.starts_with("ms-resource:") { name[known.package.len()..].to_owned() } else { own }
        } else {
            known.name.to_owned()
        };
        apps.push(App {
            name: display,
            note: known.note,
            recommended: known.recommended,
            package: name,
            removal: Removal::Packages(vec![full_name]),
        });
    }
    let resume = resume_on();
    if resume == Some(true) {
        apps.push(App {
            name: "Resume".to_owned(),
            note: RESUME_NOTE,
            recommended: false,
            package: RESUME.to_owned(),
            removal: Removal::TurnOffResume,
        });
    }
    // Two packages can be the same app under an old and a new name, such as Microsoft News.
    apps.sort_by(|a, b| b.recommended.cmp(&a.recommended).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    apps.dedup_by(|a, b| {
        let same = a.name == b.name;
        if same && let (Removal::Packages(from), Removal::Packages(into)) = (&mut a.removal, &mut b.removal) {
            into.append(from);
        }
        same
    });
    Ok(Scan { apps, guides: guides(), resume_off: resume == Some(false) })
}

// Resume.

/// Resume, by its key in the list and among the apps set aside with Ignore.
pub const RESUME: &str = "CrossDeviceResume";
const RESUME_NOTE: &str = "Continues on this PC what you had open on your phone. It is part of Windows, so it is \
                           turned off for everyone on this PC instead of removed.";
const RESUME_EXE: &str = "CrossDeviceResume.exe";
/// The policy that keeps Windows from starting CrossDeviceResume.exe, as the manifest of the package it is part
/// of names it (Connectivity/DisableCrossDeviceResume). Its definition here, with its default, shows that this
/// version of Windows has it. Setting only the default to 1 doesn't stop Windows from starting Resume at the
/// next sign-in, so the app sets the policy's current value as well, the one an MDM would set.
const RESUME_POLICY: &str = r"SOFTWARE\Microsoft\PolicyManager\default\Connectivity\DisableCrossDeviceResume";
/// The policy's current value, `DisableCrossDeviceResume`, which decides. Only administrators can write either key.
const RESUME_CURRENT: &str = r"SOFTWARE\Microsoft\PolicyManager\current\device\Connectivity";
const RESUME_VALUE: &str = "DisableCrossDeviceResume";
/// The switch for it in Settings, which on its own leaves CrossDeviceResume.exe running.
const RESUME_SETTING: &str = r"Software\Microsoft\Windows\CurrentVersion\CrossDeviceResume\Configuration";

/// Whether Resume is on: none where Windows doesn't have it.
fn resume_on() -> Option<bool> {
    let exe = system::windows_dir().join(r"SystemApps\MicrosoftWindows.Client.CBS_cw5n1h2txyewy").join(RESUME_EXE);
    if !exe.exists() || !system::reg_key_exists(Hive::LocalMachine, RESUME_POLICY) {
        return None;
    }
    Some(system::reg_dword(Hive::LocalMachine, RESUME_CURRENT, RESUME_VALUE) != Some(1))
}

/// Turns Resume on or off for the PC, and the switch in Settings with it. Windows asks for administrator rights
/// unless the app already has them; [`declined`] tells whether the user said no.
pub fn set_resume(on: bool) -> io::Result<()> {
    let data = u32::from(!on);
    // Writing straight away works only when the app runs as administrator, and only once the key exists.
    let direct = system::set_reg_dword(Hive::LocalMachine, RESUME_CURRENT, RESUME_VALUE, data)
        .and_then(|()| system::set_reg_dword(Hive::LocalMachine, RESUME_POLICY, "value", data));
    if direct.is_err() {
        set_resume_policy_as_admin(data)?;
    }
    // Settings only has the switch once Resume has started for this account; the policy is what counts.
    if system::reg_key_exists(Hive::CurrentUser, RESUME_SETTING)
        && let Err(error) = system::set_reg_dword(Hive::CurrentUser, RESUME_SETTING, "IsResumeAllowed", u32::from(on))
    {
        applog::warning("apps", format!("Couldn't set the switch for Resume in Settings: {error}"));
    }
    Ok(())
}

/// Sets the policy's current value and its default with reg.exe as administrator, without a window, so Windows
/// asks only once.
fn set_resume_policy_as_admin(data: u32) -> io::Result<()> {
    // Neither key has a space in it, so cmd.exe needs no quotes.
    let add = |key: &str, value: &str| format!(r"reg add HKLM\{key} /v {value} /t REG_DWORD /d {data} /f /reg:64");
    let parameters = format!("/c {} && {}", add(RESUME_CURRENT, RESUME_VALUE), add(RESUME_POLICY, "value"));
    match system::run_elevated(&system::system32("cmd.exe"), &parameters, Duration::from_secs(30)) {
        Ok(Some(0)) => Ok(()),
        Ok(Some(code)) => Err(io::Error::other(format!("reg.exe ended with exit code {code}"))),
        Ok(None) => Err(io::Error::other("reg.exe didn't finish within 30 seconds")),
        Err(code) => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

/// Whether setting Resume failed because the user said no to administrator rights.
pub fn declined(error: &io::Error) -> bool {
    error.raw_os_error() == Some(system::ERROR_CANCELLED as i32)
}

/// Turns Resume off and ends CrossDeviceResume.exe, which Windows then no longer starts.
fn turn_off_resume() -> Outcome {
    if let Err(error) = set_resume(false) {
        applog::warning("apps", format!("Couldn't turn off Resume: {error}"));
        return Outcome::NotRemoved(match declined(&error) {
            true => "It needs administrator rights, and Windows didn't get them.".to_owned(),
            false => format!("Windows couldn't turn it off: {error}"),
        });
    }
    // This ends the copy of this account only; copies of others signed in end when they sign out.
    let taskkill = system::system32("taskkill.exe");
    if let Err(error) = system::run_hidden(&taskkill, &["/F", "/IM", RESUME_EXE], Duration::from_secs(10)) {
        applog::warning("apps", format!("Couldn't end {RESUME_EXE}: {error}"));
    }
    Outcome::TurnedOff
}

// Removing.

const PACKAGES_IN_USE: HRESULT = HRESULT(0x80073D02_u32 as i32);
const ACCESS_DENIED: HRESULT = HRESULT(0x80070005_u32 as i32);
const PACKAGE_NOT_FOUND: HRESULT = HRESULT(0x80073CF1_u32 as i32);
const REMOVE_FAILED: HRESULT = HRESULT(0x80073CFA_u32 as i32);
const NOT_SUPPORTED: HRESULT = HRESULT(0x80070032_u32 as i32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Removed,
    /// Removed already, by something else, between the scan and now.
    AlreadyGone,
    /// Turned off, for an app Windows can't do without.
    TurnedOff,
    NotRemoved(String),
}

/// Why an app wasn't removed, in a sentence anyone can follow, with Windows' code for whoever wants it.
fn reason(code: HRESULT, extended: HRESULT, text: &str) -> String {
    let sentence = match code {
        PACKAGES_IN_USE => "It is open. Close it and try again.",
        ACCESS_DENIED => "Only an administrator can remove it.",
        REMOVE_FAILED if extended == NOT_SUPPORTED || text.contains("0x80070032") => {
            "It is part of Windows and can't be removed."
        }
        _ => "Windows couldn't remove it.",
    };
    format!("{sentence} (error 0x{:08X})", code.0 as u32)
}

/// Remove one package and wait for Windows to finish. The error is Windows' code, the extended code that
/// sometimes says more, and its own description.
fn remove_package(manager: &PackageManager, full_name: &str) -> Result<(), (HRESULT, HRESULT, String)> {
    let operation = manager
        .RemovePackageAsync(&HSTRING::from(full_name))
        .map_err(|error| (error.code(), HRESULT(0), error.message()))?;
    let outcome = operation.join();
    let code = operation.ErrorCode().unwrap_or(HRESULT(0));
    match outcome {
        Ok(result) => {
            let extended = result.ExtendedErrorCode().unwrap_or(HRESULT(0));
            let failure = if code.is_err() { code } else { extended };
            if failure.is_ok() {
                return Ok(());
            }
            Err((failure, extended, result.ErrorText().map(|text| text.to_string()).unwrap_or_default()))
        }
        Err(error) => Err((if code.is_err() { code } else { error.code() }, HRESULT(0), error.message())),
    }
}

/// Remove an app for this account.
pub fn remove(manager: &PackageManager, app: &App) -> Outcome {
    let full_names = match &app.removal {
        Removal::Packages(full_names) => full_names,
        Removal::TurnOffResume => return turn_off_resume(),
    };
    let mut removed = false;
    for full_name in full_names {
        match remove_package(manager, full_name) {
            Ok(()) => removed = true,
            Err((PACKAGE_NOT_FOUND, _, _)) => {}
            Err((code, extended, text)) => {
                applog::warning("apps", format!("Couldn't remove {full_name}: 0x{:08X} {}", code.0 as u32, text.trim()));
                return Outcome::NotRemoved(reason(code, extended, &text));
            }
        }
    }
    if removed { Outcome::Removed } else { Outcome::AlreadyGone }
}

/// Remove the apps one after another, telling `progress` which one is next, and add what happened to the log.
pub fn remove_all(apps: &[App], progress: impl Fn(usize)) -> Vec<Outcome> {
    let outcomes: Vec<Outcome> = match PackageManager::new() {
        Ok(manager) => apps
            .iter()
            .enumerate()
            .map(|(index, app)| {
                progress(index);
                remove(&manager, app)
            })
            .collect(),
        Err(error) => {
            let reason = format!("Windows couldn't start removing apps. (error 0x{:08X})", error.code().0 as u32);
            apps.iter().map(|_| Outcome::NotRemoved(reason.clone())).collect()
        }
    };
    let now = chrono::Local::now().naive_local();
    write_log(&log_text(now, apps, &outcomes));
    outcomes
}

/// What the removal said, for the window, such as "Removed 2 of 3 apps.".
pub fn result_text(outcomes: &[Outcome]) -> String {
    let total = outcomes.len();
    let failed = outcomes.iter().filter(|outcome| matches!(outcome, Outcome::NotRemoved(_))).count();
    let removed = total - failed;
    let apps = |count: usize| if count == 1 { "1 app".to_owned() } else { format!("{count} apps") };
    // Resume is the one app turned off instead.
    let turned_off = outcomes.contains(&Outcome::TurnedOff);
    match failed {
        0 if turned_off && total == 1 => "Turned off Resume.".to_owned(),
        0 if turned_off => format!("Removed {} and turned off Resume.", apps(total - 1)),
        0 => format!("Removed {}.", apps(total)),
        _ if removed == 0 => format!("Couldn't remove {}. The log says why.", if total == 1 { "it".to_owned() } else { apps(total) }),
        _ => format!("Removed {removed} of {total} apps. The log says why the others weren't."),
    }
}

// The log.

/// Where the removals are written down: removed-apps.log beside the checklist.
pub fn log_path() -> PathBuf {
    crate::state::data_file().with_file_name("removed-apps.log")
}

const LOG_LIMIT: u64 = 256 * 1024;

/// One removal, as the log keeps it: a heading, a line per app and a count.
pub fn log_text(when: NaiveDateTime, apps: &[App], outcomes: &[Outcome]) -> String {
    let mut text = format!(
        "=== {} · removing {} ===\n",
        when.format("%Y-%m-%d %H:%M"),
        if apps.len() == 1 { "1 app".to_owned() } else { format!("{} apps", apps.len()) }
    );
    let mut removed = 0;
    for (app, outcome) in apps.iter().zip(outcomes) {
        let (label, reason) = match outcome {
            Outcome::Removed => ("Removed", String::new()),
            Outcome::AlreadyGone => ("Already gone", String::new()),
            Outcome::TurnedOff => ("Turned off", String::new()),
            Outcome::NotRemoved(reason) => ("Not removed", format!(": {reason}")),
        };
        if !matches!(outcome, Outcome::NotRemoved(_)) {
            removed += 1;
        }
        text.push_str(&format!("{label:<13}{} ({}){reason}\n", app.name, app.package));
    }
    let failed = outcomes.len() - removed;
    text.push_str(&format!("{removed} removed, {failed} not removed\n\n"));
    text
}

fn write_log(text: &str) {
    if cfg!(test) {
        return;
    }
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > LOG_LIMIT) {
        let _ = std::fs::rename(&path, path.with_file_name("removed-apps.log.1"));
    }
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(text.as_bytes()));
    if let Err(error) = written {
        applog::warning("apps", format!("Couldn't write {}: {error}", path.display()));
    }
}

// What only Settings removes.

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

/// The countries of the European Economic Area, the only place Windows lets Edge be uninstalled.
const EEA: [&str; 30] = [
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "IE", "IS", "IT", "LI",
    "LT", "LU", "LV", "MT", "NL", "NO", "PL", "PT", "RO", "SE", "SI", "SK",
];

pub fn in_eea(region: &str) -> bool {
    EEA.iter().any(|country| country.eq_ignore_ascii_case(region.trim()))
}

/// The two-letter code of a Windows region number, such as "DE" for 94.
fn region_code(geo_id: i32) -> Option<String> {
    use windows_sys::Win32::Globalization::{GEO_ISO2, GetGeoInfoW};
    let mut buffer = [0u16; 8];
    let length = unsafe { GetGeoInfoW(geo_id, GEO_ISO2, buffer.as_mut_ptr(), buffer.len() as i32, 0) };
    (length > 1).then(|| system::from_wide(&buffer))
}

/// The region the PC was set up for, which decides whether Edge can be uninstalled; changing the region
/// afterwards doesn't. Falls back on the region chosen in Settings.
fn device_region() -> Option<String> {
    let key = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\DeviceRegion";
    system::reg_dword(Hive::LocalMachine, key, "DeviceRegion")
        .and_then(|id| region_code(id as i32))
        .or_else(|| system::reg_string(Hive::CurrentUser, r"Control Panel\International\Geo", "Name"))
}

const OPEN_INSTALLED_APPS: &str = "Open Installed apps, find {name}, select the three dots (…) next to it and then Uninstall.";

fn steps(name: &str) -> String {
    OPEN_INSTALLED_APPS.replace("{name}", name)
}

/// Trial software that PC makers put on new PCs, by the start of its name in the list of installed programs.
const TRIALS: [(&str, &str); 5] = [
    ("McAfee", "Microsoft Defender protects the PC again once it is gone."),
    ("Norton", "Microsoft Defender protects the PC again once it is gone."),
    ("WildTangent", ""),
    ("Dropbox Promotion", ""),
    ("ExpressVPN", "Keep it if you pay for it."),
];

fn installed(display_names: &mut Vec<String>, hive: Hive, key: &str) {
    for subkey in system::reg_subkeys(hive, key) {
        let path = format!(r"{key}\{subkey}");
        if system::reg_dword(hive, &path, "SystemComponent") == Some(1)
            || system::reg_string(hive, &path, "ParentKeyName").is_some()
        {
            continue;
        }
        if let Some(name) = system::reg_string(hive, &path, "DisplayName").filter(|name| !name.trim().is_empty()) {
            display_names.push(name.trim().to_owned());
        }
    }
}

/// The apps on the PC that only Settings can remove: Edge where Windows allows it, OneDrive and trials.
pub fn guides() -> Vec<Guide> {
    let mut guides = Vec::new();
    let edge = [(Hive::LocalMachine, UNINSTALL_32), (Hive::LocalMachine, UNINSTALL), (Hive::CurrentUser, UNINSTALL)]
        .iter()
        .any(|(hive, key)| system::reg_key_exists(*hive, &format!(r"{key}\Microsoft Edge")));
    if edge && device_region().is_none_or(|region| in_eea(&region)) {
        guides.push(Guide {
            name: "Microsoft Edge".to_owned(),
            steps: format!("Install another browser first and make it your default. Then {}", lowercase_first(&steps("Microsoft Edge"))),
            note: "Windows only offers this in the European Economic Area. Apps that show web pages keep working.",
        });
    }
    let onedrive = [(Hive::CurrentUser, UNINSTALL), (Hive::LocalMachine, UNINSTALL), (Hive::LocalMachine, UNINSTALL_32)]
        .iter()
        .any(|(hive, key)| system::reg_key_exists(*hive, &format!(r"{key}\OneDriveSetup.exe")));
    if onedrive {
        guides.push(Guide {
            name: "Microsoft OneDrive".to_owned(),
            steps: steps("Microsoft OneDrive"),
            note: "Files that are only online then aren't on this PC, but they stay safe on onedrive.com.",
        });
    }
    let mut display_names = Vec::new();
    installed(&mut display_names, Hive::LocalMachine, UNINSTALL);
    installed(&mut display_names, Hive::LocalMachine, UNINSTALL_32);
    installed(&mut display_names, Hive::CurrentUser, UNINSTALL);
    display_names.sort_by_key(|name| name.to_lowercase());
    display_names.dedup();
    for name in display_names {
        if let Some((_, note)) = TRIALS.iter().find(|(start, _)| name.starts_with(start)) {
            guides.push(Guide { steps: steps(&name), name, note });
        }
    }
    guides
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, package: &str) -> App {
        App {
            name: name.to_owned(),
            note: "",
            recommended: true,
            package: package.to_owned(),
            removal: Removal::Packages(Vec::new()),
        }
    }

    #[test]
    fn what_windows_needs_and_everyday_apps_are_never_offered() {
        for package in [
            "Microsoft.WindowsCalculator",
            "Microsoft.Windows.Photos",
            "Microsoft.ScreenSketch",
            "Microsoft.WindowsNotepad",
            "Microsoft.Paint",
            "Microsoft.ZuneMusic",
            "Microsoft.WindowsCamera",
            "Microsoft.WindowsTerminal",
            "Microsoft.WindowsStore",
            "Microsoft.DesktopAppInstaller",
            "Microsoft.SecHealthUI",
            "Microsoft.XboxIdentityProvider",
            "Microsoft.Xbox.TCUI",
            "Microsoft.GamingServices",
            "Microsoft.WidgetsPlatformRuntime",
            "MicrosoftWindows.CrossDevice",
            "Microsoft.WebMediaExtensions",
            "Microsoft.VCLibs.140.00",
            "king.com.",
        ] {
            assert!(known(package).is_none(), "{package} must not be offered");
        }
    }

    #[test]
    fn apps_are_found_by_their_package_name() {
        assert_eq!(known("Microsoft.MSPaint").map(|known| known.name), Some("Paint 3D"));
        assert_eq!(known("microsoft.bingnews").map(|known| known.name), Some("Microsoft News"));
        assert!(known("king.com.CandyCrushSaga").is_some_and(|known| known.recommended && known.name.is_empty()));
        assert!(known("Microsoft.GamingApp").is_some_and(|known| !known.recommended));
    }

    #[test]
    fn the_list_has_every_package_once() {
        for (index, known) in CATALOG.iter().enumerate() {
            assert!(!known.package.is_empty());
            assert!(!known.name.is_empty() || known.package.ends_with('.'), "{} needs a name", known.package);
            assert!(
                CATALOG[index + 1..].iter().all(|other| !other.package.eq_ignore_ascii_case(known.package)),
                "{} is listed twice",
                known.package
            );
        }
    }

    #[test]
    fn reasons_are_plain_sentences_with_the_code() {
        let none = HRESULT(0);
        assert_eq!(reason(PACKAGES_IN_USE, none, ""), "It is open. Close it and try again. (error 0x80073D02)");
        assert_eq!(
            reason(REMOVE_FAILED, none, "error 0x80070032: AppX Deployment Remove operation failed."),
            "It is part of Windows and can't be removed. (error 0x80073CFA)"
        );
        assert_eq!(reason(REMOVE_FAILED, NOT_SUPPORTED, ""), "It is part of Windows and can't be removed. (error 0x80073CFA)");
        assert_eq!(reason(HRESULT(0x80073CF9_u32 as i32), none, "Install failed."), "Windows couldn't remove it. (error 0x80073CF9)");
    }

    #[test]
    fn the_log_has_a_line_per_app_and_a_count() {
        let when = chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(10, 21, 0).unwrap();
        let apps = [app("Microsoft News", "Microsoft.BingNews"), app("Tips", "Microsoft.Getstarted"), app("Solitaire Collection", "Microsoft.MicrosoftSolitaireCollection")];
        let outcomes =
            [Outcome::Removed, Outcome::AlreadyGone, Outcome::NotRemoved(reason(PACKAGES_IN_USE, HRESULT(0), ""))];
        assert_eq!(
            log_text(when, &apps, &outcomes),
            "=== 2026-11-08 10:21 · removing 3 apps ===\n\
             Removed      Microsoft News (Microsoft.BingNews)\n\
             Already gone Tips (Microsoft.Getstarted)\n\
             Not removed  Solitaire Collection (Microsoft.MicrosoftSolitaireCollection): It is open. Close it and try again. (error 0x80073D02)\n\
             2 removed, 1 not removed\n\n"
        );
    }

    #[test]
    fn the_result_says_how_many_went() {
        let failed = || Outcome::NotRemoved("x".to_owned());
        assert_eq!(result_text(&[Outcome::Removed, Outcome::AlreadyGone]), "Removed 2 apps.");
        assert_eq!(result_text(&[Outcome::Removed]), "Removed 1 app.");
        assert_eq!(result_text(&[failed()]), "Couldn't remove it. The log says why.");
        assert_eq!(result_text(&[failed(), failed()]), "Couldn't remove 2 apps. The log says why.");
        assert_eq!(result_text(&[Outcome::Removed, failed(), Outcome::Removed]), "Removed 2 of 3 apps. The log says why the others weren't.");
        assert_eq!(result_text(&[Outcome::TurnedOff]), "Turned off Resume.");
        assert_eq!(result_text(&[Outcome::Removed, Outcome::TurnedOff]), "Removed 1 app and turned off Resume.");
    }

    #[test]
    fn the_summary_counts_both_kinds() {
        let mut scan = Scan::default();
        assert_eq!(scan.summary(), "Nothing left to remove");
        assert_eq!(scan.breakdown(), "");
        scan.apps = vec![app("Tips", "Microsoft.Getstarted"), App { recommended: false, ..app("Clock", "Microsoft.WindowsAlarms") }];
        scan.guides = vec![Guide { name: "Microsoft Edge".to_owned(), steps: String::new(), note: "" }];
        assert_eq!(scan.summary(), "3 apps can still be removed");
        assert_eq!(scan.breakdown(), "1 recommended · 1 more here · 1 in Settings");
        scan.apps.remove(0);
        assert_eq!(scan.breakdown(), "1 here · 1 in Settings");
        scan.guides.clear();
        assert_eq!(scan.breakdown(), "");
    }

    #[test]
    fn ignored_apps_are_neither_offered_nor_counted() {
        let scan = Scan {
            apps: vec![app("Tips", "Microsoft.Getstarted"), App { recommended: false, ..app("Clock", "Microsoft.WindowsAlarms") }],
            guides: vec![Guide { name: "Microsoft OneDrive".to_owned(), steps: String::new(), note: "" }],
            resume_off: false,
        };
        let ignored: BTreeSet<String> = ["Microsoft.WindowsAlarms", "Microsoft OneDrive", "Microsoft.BingWeather"].map(str::to_owned).into();
        let offered = scan.without(&ignored);
        assert_eq!(offered.apps, vec![app("Tips", "Microsoft.Getstarted")]);
        assert!(offered.guides.is_empty());
        assert_eq!(offered.summary(), "1 app can still be removed");
        // Only what is still installed is listed as ignored.
        let listed = [("Microsoft.WindowsAlarms", "Clock"), ("Microsoft OneDrive", "Microsoft OneDrive")];
        assert_eq!(scan.ignored(&ignored), listed.map(|(key, name)| (key.to_owned(), name.to_owned())));
        assert_eq!(scan.without(&BTreeSet::new()), scan);
    }

    /// Lists what this PC has, without removing anything: `cargo test scan_this_pc -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scan_this_pc() {
        let scan = scan().unwrap();
        for app in &scan.apps {
            println!("{:<28} {:<45} {} {:?}", app.name, app.package, app.recommended, app.removal);
        }
        for guide in &scan.guides {
            println!("{}: {} {}", guide.name, guide.steps, guide.note);
        }
        println!("Resume turned off: {}", scan.resume_off);
        println!("{} · {}", scan.summary(), scan.breakdown());
    }

    #[test]
    fn edge_can_only_go_in_the_eea() {
        assert!(in_eea("DE") && in_eea("no ") && in_eea("is"));
        assert!(!in_eea("US") && !in_eea("GB") && !in_eea("CH") && !in_eea(""));
        assert_eq!(region_code(94).as_deref(), Some("DE"));
        assert_eq!(region_code(244).as_deref(), Some("US"));
    }
}
