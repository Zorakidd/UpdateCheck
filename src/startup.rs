//! Start-up before the window opens: one copy at a time, the self-update, starting with Windows, and whether a
//! background check reminds at all.
//!
//! A background check, which Windows runs at every sign-in and once a day, usually finds nothing due and ends
//! here without opening a window or touching the network.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use chrono::{SecondsFormat, TimeDelta};

use crate::applog;
use crate::state::{self, Now, State};
use crate::system;
use crate::updater::{self, Outcome};

/// How long a start waits before asking GitHub about a new release again. Releases are rare, and most starts
/// are then spared the round trip.
const UPDATE_INTERVAL: TimeDelta = TimeDelta::days(1);
const MB_ICONERROR: u32 = 0x10;

/// What start-up found to do.
pub enum Start {
    /// Nothing more: another copy is open and was brought to the front if wanted, or no reminder is due.
    Done,
    /// A newer version was installed, to be started in this one's place.
    Restart,
    /// Open the window, as a reminder or because it was started by hand.
    Window { state: State, reminder: bool },
}

/// What start-up asks of Windows and GitHub, which the tests stand in for.
pub trait Host {
    /// Whether this is the build users run. Only that one registers with Windows and updates itself, so a
    /// development build never points Windows at itself.
    fn release(&self) -> bool {
        system::is_release()
    }

    /// Whether the app may update itself, which it may not where an organization rolls out its versions.
    fn self_update_allowed(&self) -> bool {
        system::self_update_allowed()
    }

    fn update_due(&mut self, data_dir: &Path, now: &Now) -> bool {
        update_due(data_dir, now)
    }

    fn self_update(&mut self, data_dir: &Path) -> Outcome {
        updater::self_update(data_dir)
    }

    fn sync_autostart(&mut self, enabled: bool, hour: u32) -> Result<(), String> {
        system::sync_autostart(enabled, hour)
    }

    fn user_is_busy(&self) -> bool {
        system::user_is_busy()
    }

    fn focus_window(&mut self) {
        system::focus_window(system::TITLE);
    }
}

/// The real thing.
pub struct Windows;

impl Host for Windows {}

/// Start the app: open the checklist, or with `check_only` only when this week's check still needs a reminder.
/// `open` shows the window and returns when it is closed.
pub fn run(check_only: bool, open: impl FnOnce(State, PathBuf, bool) -> Result<(), String>) -> ExitCode {
    let path = state::data_file();
    let now = state::local_now();
    let lock = system::claim_single_instance();
    match prepare(&mut Windows, &path, &now, check_only, lock.is_some()) {
        Ok(Start::Done) => ExitCode::SUCCESS,
        Ok(Start::Restart) => {
            // The new version must find the lock free, or it would take itself for a second copy.
            drop(lock);
            updater::restart();
            ExitCode::SUCCESS
        }
        Ok(Start::Window { state, reminder }) => {
            let result = open(state, path, reminder);
            drop(lock);
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => fail(false, &format!("Couldn't open the window: {error}")),
            }
        }
        Err(error) => fail(check_only, &error.to_string()),
    }
}

/// Log an error that stopped the app and, where someone is waiting for the window, tell them where to find it.
/// A background check stays quiet: nobody started it.
pub fn fail(check_only: bool, error: &str) -> ExitCode {
    applog::error("startup", format!("Update Check stopped on an unexpected error: {error}"));
    if system::is_release() && !check_only {
        let message = format!(
            "Update Check couldn't start because of an unexpected error. The details are in\n{}",
            applog::log_file().display()
        );
        system::message_box(&message, MB_ICONERROR);
    }
    ExitCode::from(1)
}

/// Everything up to opening the window. `first_copy` is whether this copy holds the single-instance lock.
pub fn prepare(host: &mut impl Host, path: &Path, now: &Now, check_only: bool, first_copy: bool) -> io::Result<Start> {
    if !first_copy {
        // The open window saves its own checklist, so it is only read here, to see whether a reminder is due.
        let due = load(path).map(|mut state| {
            state.roll_over(now);
            state.should_remind(now)
        });
        if !check_only || (due.unwrap_or(false) && !host.user_is_busy()) {
            host.focus_window();
        }
        return Ok(Start::Done);
    }

    let data_dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(data_dir)?;
    // Holding the lock means no other copy runs from the .exe that an update replaces. Once a day is often
    // enough to ask.
    if host.release() && host.self_update_allowed() && host.update_due(data_dir, now)
        && host.self_update(data_dir) == Outcome::Installed {
            return Ok(Start::Restart);
        }

    let mut state = load(path)?;
    // Nothing is registered before the first start has asked.
    if let Some(enabled) = state.settings.autostart.filter(|_| host.release())
        && let Err(error) = host.sync_autostart(enabled, state.settings.hour) {
            applog::error("startup", format!("Couldn't register the app to start with Windows: {error}"));
        }
    state.roll_over(now);
    if check_only {
        // A full-screen game or a presentation isn't interrupted. Today's reminder then waits for the next
        // sign-in or daily check.
        if !state.should_remind(now) || host.user_is_busy() {
            save(&state, path);
            return Ok(Start::Done);
        }
        state.last_reminded = Some(now.date_naive());
    }
    save(&state, path);
    Ok(Start::Window { state, reminder: check_only })
}

/// The checklist, read again after a moment if another program has the file open, as a virus scanner or a
/// backup does for a moment.
fn load(path: &Path) -> io::Result<State> {
    for _ in 0..3 {
        match state::load(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => std::thread::sleep(Duration::from_millis(150)),
            result => return result,
        }
    }
    state::load(path)
}

/// Saved where it can be. A checklist that can't be saved now is saved by the window once something changes,
/// so this is no reason not to open it.
fn save(state: &State, path: &Path) {
    if let Err(error) = state::save(state, path) {
        applog::warning("startup", format!("Couldn't save the checklist: {error}"));
    }
}

/// Whether this start should ask GitHub for a newer release. The time is noted before the question is asked,
/// so a failed check waits for the next day instead of trying again at every sign-in.
pub fn update_due(data_dir: &Path, now: &Now) -> bool {
    let marker = data_dir.join("last-checked.txt");
    let asked = std::fs::read_to_string(&marker).ok().and_then(|text| state::parse_time(text.trim()));
    if asked.is_some_and(|asked| *now - asked < UPDATE_INTERVAL) {
        return false;
    }
    let _ = std::fs::write(&marker, now.to_rfc3339_opts(SecondsFormat::Micros, false));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Settings, local_now, new_id};

    #[derive(Default)]
    struct Fake {
        busy: bool,
        policy_forbids_updates: bool,
        update: Option<Outcome>,
        asked: usize,
        registered: Vec<bool>,
        focused: usize,
    }

    impl Host for Fake {
        fn release(&self) -> bool {
            true
        }

        fn self_update_allowed(&self) -> bool {
            !self.policy_forbids_updates
        }

        fn update_due(&mut self, _: &Path, _: &Now) -> bool {
            self.asked += 1;
            self.update.is_some()
        }

        fn self_update(&mut self, _: &Path) -> Outcome {
            self.update.unwrap_or(Outcome::UpToDate)
        }

        fn sync_autostart(&mut self, enabled: bool, _: u32) -> Result<(), String> {
            self.registered.push(enabled);
            Ok(())
        }

        fn user_is_busy(&self) -> bool {
            self.busy
        }

        fn focus_window(&mut self) {
            self.focused += 1;
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("update-check-test-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn opened(start: &Start) -> Option<bool> {
        match start {
            Start::Window { reminder, .. } => Some(*reminder),
            _ => None,
        }
    }

    fn saved(path: &Path, autostart: Option<bool>) {
        let settings = Settings { autostart, ..Settings::default() };
        state::save(&State { settings, ..State::default() }, path).unwrap();
    }

    #[test]
    fn the_first_start_sets_nothing_up_before_asking() {
        let path = temp_dir("first").join("data").join("state.json");
        let mut host = Fake::default();
        let start = prepare(&mut host, &path, &local_now(), false, true).unwrap();
        assert!(host.registered.is_empty());
        // The window asks; started by hand, not as a reminder.
        assert_eq!(opened(&start), Some(false));
        assert!(path.exists());
    }

    #[test]
    fn later_starts_apply_the_answer() {
        let path = temp_dir("later").join("state.json");
        saved(&path, Some(false));
        let mut host = Fake::default();
        prepare(&mut host, &path, &local_now(), false, true).unwrap();
        assert_eq!(host.registered, [false]);
    }

    #[test]
    fn where_an_administrator_rolls_out_versions_github_isnt_asked() {
        let path = temp_dir("policy").join("state.json");
        let mut host = Fake { policy_forbids_updates: true, ..Fake::default() };
        prepare(&mut host, &path, &local_now(), false, true).unwrap();
        assert_eq!(host.asked, 0);
    }

    #[test]
    fn an_installed_update_restarts_and_a_failed_one_carries_on() {
        let path = temp_dir("update").join("state.json");
        let mut host = Fake { update: Some(Outcome::Installed), ..Fake::default() };
        assert!(matches!(prepare(&mut host, &path, &local_now(), false, true).unwrap(), Start::Restart));
        let mut host = Fake { update: Some(Outcome::Failed), ..Fake::default() };
        assert_eq!(opened(&prepare(&mut host, &path, &local_now(), false, true).unwrap()), Some(false));
    }

    #[test]
    fn a_reminder_waits_while_a_game_or_presentation_is_full_screen() {
        let path = temp_dir("busy").join("state.json");
        saved(&path, Some(true));
        let mut host = Fake { busy: true, ..Fake::default() };
        assert!(matches!(prepare(&mut host, &path, &local_now(), true, true).unwrap(), Start::Done));
        // Still due, so the next check reminds, and the one after that doesn't again that day.
        let mut host = Fake::default();
        assert_eq!(opened(&prepare(&mut host, &path, &local_now(), true, true).unwrap()), Some(true));
        assert!(state::load(&path).unwrap().last_reminded.is_some());
        assert!(matches!(prepare(&mut host, &path, &local_now(), true, true).unwrap(), Start::Done));
    }

    #[test]
    fn a_second_copy_brings_the_first_to_the_front_only_when_asked_to() {
        let path = temp_dir("second").join("state.json");
        saved(&path, Some(true));
        let mut host = Fake::default();
        assert!(matches!(prepare(&mut host, &path, &local_now(), false, false).unwrap(), Start::Done));
        assert_eq!(host.focused, 1);
        // A background check with a reminder due shows the open window too, and one without leaves it alone.
        prepare(&mut host, &path, &local_now(), true, false).unwrap();
        assert_eq!(host.focused, 2);
        let mut state = state::load(&path).unwrap();
        state.last_reminded = Some(local_now().date_naive());
        state::save(&state, &path).unwrap();
        prepare(&mut host, &path, &local_now(), true, false).unwrap();
        assert_eq!(host.focused, 2);
        // The second copy changes nothing, nor does it register anything.
        assert!(host.registered.is_empty());
        assert_eq!(host.asked, 0);
    }

    #[test]
    fn the_first_run_asks_about_a_new_release_and_the_next_one_waits_a_day() {
        let dir = temp_dir("due");
        let now = local_now();
        assert!(update_due(&dir, &now));
        assert!(!update_due(&dir, &(now + UPDATE_INTERVAL - TimeDelta::seconds(1))));
        assert!(update_due(&dir, &(now + UPDATE_INTERVAL)));
    }

    #[test]
    fn an_unreadable_marker_does_not_stop_the_update_check() {
        let dir = temp_dir("marker");
        std::fs::write(dir.join("last-checked.txt"), "not a timestamp").unwrap();
        assert!(update_due(&dir, &local_now()));
        // What the first versions wrote.
        std::fs::write(dir.join("last-checked.txt"), local_now().naive_local().format("%Y-%m-%dT%H:%M:%S%.6f").to_string()).unwrap();
        assert!(!update_due(&dir, &local_now()));
    }
}
