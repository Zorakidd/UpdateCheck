//! The checklist, its settings and its progress, and how they are stored.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Timelike};
use serde_json::{Map, Value, json};

/// A point in time with the UTC offset it was taken in. The week is worked out in local wall-clock time, so a
/// change to or from daylight saving time doesn't move it.
pub type Now = DateTime<FixedOffset>;

pub fn local_now() -> Now {
    Local::now().fixed_offset()
}

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const APPEARANCES: [&str; 3] = ["System", "Light", "Dark"];

pub const WINDOWS_UPDATE_STEP: &str = "windows-update";
pub const DRIVERS_STEP: &str = "drivers";
pub const GRAPHICS_STEP: &str = "graphics";
pub const WINGET_STEP: &str = "winget";
pub const STORAGE_STEP: &str = "storage";
pub const APPS_STEP: &str = "apps";
pub const DEFENDER_STEP: &str = "defender";

/// The default steps of the first version, which every checklist saved without `offered_steps` was offered.
const ORIGINAL_DEFAULT_STEPS: [&str; 6] = ["windows-update", "drivers", "store", "winget", "defender", "backup"];
/// How many finished weeks are remembered for the streak: five years.
const WEEKS_KEPT: usize = 260;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    pub detail: String,
    pub action: String,
    pub id: String,
}

impl Step {
    pub fn new(title: &str, detail: &str, action: &str) -> Self {
        Step { title: title.to_owned(), detail: detail.to_owned(), action: action.to_owned(), id: new_id() }
    }

    fn with_id(title: &str, detail: &str, action: &str, id: &str) -> Self {
        Step { title: title.to_owned(), detail: detail.to_owned(), action: action.to_owned(), id: id.to_owned() }
    }
}

/// A short random id for a step added by hand.
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_i64(Local::now().timestamp_nanos_opt().unwrap_or_default());
    format!("{:08x}", hasher.finish() as u32)
}

pub fn default_steps() -> Vec<Step> {
    vec![
        Step::with_id(
            "Windows Update",
            "Install pending updates, then restart if asked.",
            "ms-settings:windowsupdate",
            WINDOWS_UPDATE_STEP,
        ),
        Step::with_id(
            "Optional driver updates",
            "Review optional updates for drivers and firmware.",
            "ms-settings:windowsupdate-optionalupdates",
            DRIVERS_STEP,
        ),
        Step::with_id("Graphics drivers", "", "", GRAPHICS_STEP),
        Step::with_id(
            "Microsoft Store apps",
            "Open the Library and click \u{201c}Get updates\u{201d}.",
            "ms-windows-store://downloadsandupdates",
            "store",
        ),
        Step::with_id("Desktop apps", "Upgrade everything winget manages.", "winget upgrade --all", WINGET_STEP),
        Step::with_id("Microsoft Defender", "Check for new protection updates.", "windowsdefender://threat", "defender"),
        Step::with_id(
            "Free up space",
            "Delete temporary files, or turn on Storage Sense to do it automatically.",
            "ms-settings:storagesense",
            STORAGE_STEP,
        ),
        Step::with_id("Preinstalled apps", "Remove trial software and apps you don't use.", "", APPS_STEP),
        Step::with_id("Backups", "Make sure the latest backup finished without errors.", "", "backup"),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// 0 is Monday.
    pub weekday: u32,
    pub hour: u32,
    pub appearance: String,
    /// Whether to start with Windows and remind, or None while the first start hasn't asked yet.
    pub autostart: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { weekday: 0, hour: 10, appearance: "System".to_owned(), autostart: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub steps: Vec<Step>,
    pub settings: Settings,
    /// The first day of the week the ticks in `done` belong to.
    pub cycle_start: Option<NaiveDate>,
    pub done: BTreeSet<String>,
    pub last_completed: Option<Now>,
    pub last_reminded: Option<NaiveDate>,
    /// The default steps this checklist has been given, so a default step the user removed doesn't come back.
    pub offered_steps: BTreeSet<String>,
    pub last_done: BTreeMap<String, NaiveDate>,
    /// The first day of each week whose checklist was finished, oldest first.
    pub completed_weeks: Vec<NaiveDate>,
    /// The apps the Apps tab no longer offers or counts, by package, or by name for one only Settings removes.
    pub ignored_apps: BTreeSet<String>,
}

impl Default for State {
    fn default() -> Self {
        let steps = default_steps();
        let offered_steps = steps.iter().map(|step| step.id.clone()).collect();
        State {
            steps,
            settings: Settings::default(),
            cycle_start: None,
            done: BTreeSet::new(),
            last_completed: None,
            last_reminded: None,
            offered_steps,
            last_done: BTreeMap::new(),
            completed_weeks: Vec::new(),
            ignored_apps: BTreeSet::new(),
        }
    }
}

impl State {
    #[cfg(test)]
    pub fn with_steps(steps: Vec<Step>, settings: Settings) -> Self {
        State { steps, settings, ..State::default() }
    }

    pub fn is_complete(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|step| self.done.contains(&step.id))
    }

    pub fn done_count(&self) -> usize {
        self.steps.iter().filter(|step| self.done.contains(&step.id)).count()
    }

    /// The day the current week began: the latest reminder day whose reminder hour has passed.
    pub fn current_cycle_start(&self, now: &Now) -> NaiveDate {
        let mut back = (now.weekday().num_days_from_monday() + 7 - self.settings.weekday % 7) % 7;
        if back == 0 && now.hour() < self.settings.hour {
            back = 7;
        }
        now.date_naive() - Duration::days(i64::from(back))
    }

    /// The day the next week begins, at the reminder hour.
    pub fn next_reminder(&self, now: &Now) -> NaiveDate {
        self.current_cycle_start(now) + Duration::days(7)
    }

    /// Start a new week if one has begun. Returns whether it did.
    pub fn roll_over(&mut self, now: &Now) -> bool {
        let start = self.current_cycle_start(now);
        if self.cycle_start.is_some_and(|current| start <= current) {
            return false;
        }
        self.cycle_start = Some(start);
        self.done.clear();
        true
    }

    pub fn set_done(&mut self, step_id: &str, done: bool, now: &Now) {
        if done {
            self.done.insert(step_id.to_owned());
            self.last_done.insert(step_id.to_owned(), now.date_naive());
        } else {
            // A correction to this week, not a claim that it was never done: last_done stays.
            self.done.remove(step_id);
        }
        if self.is_complete() {
            self.last_completed = Some(*now);
            if let Some(start) = self.cycle_start
                && !self.completed_weeks.contains(&start)
            {
                self.completed_weeks.push(start);
                let excess = self.completed_weeks.len().saturating_sub(WEEKS_KEPT);
                self.completed_weeks.drain(..excess);
            }
        } else if self.cycle_start.is_some() && self.completed_weeks.last() == self.cycle_start.as_ref() {
            self.completed_weeks.pop();
        }
    }

    /// How many weeks in a row were finished, counting this one once it is. Weeks are matched by the days
    /// they span rather than their first day, so moving the reminder day keeps the streak.
    pub fn streak(&self, now: &Now) -> u32 {
        let finished = |start: NaiveDate| {
            self.completed_weeks.iter().any(|week| start <= *week && *week < start + Duration::days(7))
        };
        let mut start = self.current_cycle_start(now);
        if !finished(start) {
            start -= Duration::days(7);
        }
        let mut count = 0;
        while finished(start) {
            count += 1;
            start -= Duration::days(7);
        }
        count
    }

    /// Whether to open the window for this week's checklist: at most once a day, until it is finished.
    pub fn should_remind(&self, now: &Now) -> bool {
        !self.steps.is_empty() && !self.is_complete() && self.last_reminded != Some(now.date_naive())
    }

    /// Give the checklist the default steps added since it was saved, each once, next to the default steps
    /// that come before it.
    pub fn add_new_default_steps(&mut self) {
        let defaults = default_steps();
        for (index, step) in defaults.iter().enumerate() {
            if !self.offered_steps.insert(step.id.clone()) || self.steps.iter().any(|own| own.id == step.id) {
                continue;
            }
            let earlier: Vec<&str> = defaults[..index].iter().map(|default| default.id.as_str()).collect();
            let at = self
                .steps
                .iter()
                .rposition(|own| earlier.contains(&own.id.as_str()))
                .map_or(self.steps.len(), |position| position + 1);
            self.steps.insert(at, step.clone());
        }
    }

    pub fn to_json(&self) -> Value {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|step| json!({"title": step.title, "detail": step.detail, "action": step.action, "id": step.id}))
            .collect();
        let date = |day: &Option<NaiveDate>| day.map_or(Value::Null, |day| Value::String(day.to_string()));
        let mut last_done = Map::new();
        for (id, day) in &self.last_done {
            last_done.insert(id.clone(), Value::String(day.to_string()));
        }
        json!({
            "steps": steps,
            "settings": {
                "weekday": self.settings.weekday,
                "hour": self.settings.hour,
                "appearance": self.settings.appearance,
                "autostart": self.settings.autostart,
            },
            "cycle_start": date(&self.cycle_start),
            "done": self.done.iter().collect::<Vec<_>>(),
            "last_completed": self
                .last_completed
                .map_or(Value::Null, |when| Value::String(when.to_rfc3339_opts(SecondsFormat::Micros, false))),
            "last_reminded": date(&self.last_reminded),
            "offered_steps": self.offered_steps.iter().collect::<Vec<_>>(),
            "last_done": last_done,
            "completed_weeks": self.completed_weeks.iter().map(|day| day.to_string()).collect::<Vec<_>>(),
            "ignored_apps": self.ignored_apps.iter().collect::<Vec<_>>(),
        })
    }

    /// Read a saved checklist. Whatever is missing or doesn't make sense falls back to its default, so one odd
    /// value doesn't cost the whole checklist.
    pub fn from_json(data: &Map<String, Value>) -> State {
        let text = |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or_default().to_owned();
        let steps = data
            .get("steps")
            .and_then(Value::as_array)
            .map(|steps| {
                steps
                    .iter()
                    .filter_map(Value::as_object)
                    .filter_map(|step| {
                        let title = step.get("title")?.as_str()?.trim();
                        if title.is_empty() {
                            return None;
                        }
                        let id = text(step.get("id"));
                        Some(Step {
                            title: title.to_owned(),
                            detail: text(step.get("detail")),
                            action: text(step.get("action")),
                            id: if id.is_empty() { new_id() } else { id },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let defaults = Settings::default();
        let settings = data.get("settings").and_then(Value::as_object);
        let number = |name: &str, below: u64, default: u32| {
            settings
                .and_then(|settings| settings.get(name))
                .and_then(Value::as_u64)
                .filter(|value| *value < below)
                .map_or(default, |value| value as u32)
        };
        let appearance = settings
            .and_then(|settings| settings.get("appearance"))
            .and_then(Value::as_str)
            .filter(|appearance| APPEARANCES.contains(appearance))
            .unwrap_or(&defaults.appearance)
            .to_owned();
        let autostart = settings.and_then(|settings| settings.get("autostart")).and_then(Value::as_bool);
        let settings = Settings {
            weekday: number("weekday", 7, defaults.weekday),
            hour: number("hour", 24, defaults.hour),
            appearance,
            autostart,
        };

        let strings = |name: &str| -> Option<Vec<String>> {
            data.get(name)?.as_array().map(|items| {
                items.iter().filter_map(Value::as_str).map(str::to_owned).collect()
            })
        };
        let offered_steps = strings("offered_steps")
            .unwrap_or_else(|| ORIGINAL_DEFAULT_STEPS.iter().map(|id| (*id).to_owned()).collect())
            .into_iter()
            .collect();
        let last_done = data
            .get("last_done")
            .and_then(Value::as_object)
            .map(|days| {
                days.iter().filter_map(|(id, day)| Some((id.clone(), parse_date(day.as_str()?)?))).collect()
            })
            .unwrap_or_default();
        let mut completed_weeks: Vec<NaiveDate> =
            strings("completed_weeks").unwrap_or_default().iter().filter_map(|day| parse_date(day)).collect();
        completed_weeks.sort();
        completed_weeks.dedup();

        let mut state = State {
            steps,
            settings,
            cycle_start: data.get("cycle_start").and_then(Value::as_str).and_then(parse_date),
            done: strings("done").unwrap_or_default().into_iter().collect(),
            last_completed: data.get("last_completed").and_then(Value::as_str).and_then(parse_time),
            last_reminded: data.get("last_reminded").and_then(Value::as_str).and_then(parse_date),
            offered_steps,
            last_done,
            completed_weeks,
            ignored_apps: strings("ignored_apps").unwrap_or_default().into_iter().collect(),
        };
        state.add_new_default_steps();
        state
    }
}

/// A day saved as "2026-09-14", or as the start of a timestamp.
fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

pub fn parse_time(text: &str) -> Option<Now> {
    DateTime::parse_from_rfc3339(text).ok().or_else(|| {
        let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
        Local.from_local_datetime(&naive).earliest().map(|when| when.fixed_offset())
    })
}

/// Where the checklist is kept: `UpdateCheck\state.json` in the Downloads folder, or the file
/// `UPDATE_CHECK_DATA` names. The logs go beside it.
pub fn data_file() -> PathBuf {
    if let Some(path) = std::env::var_os("UPDATE_CHECK_DATA").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    static DEFAULT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DEFAULT
        .get_or_init(|| {
            let folder = crate::system::downloads_dir().unwrap_or_else(|| PathBuf::from(".")).join("UpdateCheck");
            // Earlier versions kept it in %APPDATA%\UpdateCheck. Tests leave that folder alone.
            if !cfg!(test)
                && let Some(appdata) = std::env::var_os("APPDATA")
            {
                move_folder(&PathBuf::from(appdata).join("UpdateCheck"), &folder);
            }
            folder.join("state.json")
        })
        .clone()
}

/// Moves the old data folder to the new place, once: only while the new one doesn't exist yet. The old one is
/// deleted only when every file in it was copied.
fn move_folder(old: &std::path::Path, new: &std::path::Path) {
    if new.exists() || !old.is_dir() || std::fs::rename(old, new).is_ok() {
        return;
    }
    // A rename can't reach another drive, and fails while another copy of the app has a file open.
    let Ok(entries) = std::fs::read_dir(old) else { return };
    if std::fs::create_dir_all(new).is_err() {
        return;
    }
    let mut copied = true;
    for entry in entries.flatten() {
        if entry.path().is_file() {
            copied &= std::fs::copy(entry.path(), new.join(entry.file_name())).is_ok();
        }
    }
    if copied {
        let _ = std::fs::remove_dir_all(old);
    }
}

/// The checklist saved at `path`, or the default one if there is none. A file that can't be read as a
/// checklist is kept beside it as state.json.bak. Any other error, such as a file locked by another program,
/// is returned, so a checklist that is there is never overwritten with the default one.
pub fn load(path: &Path) -> io::Result<State> {
    let text = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(error) => return Err(error),
    };
    let parsed = std::str::from_utf8(text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&text))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    match parsed {
        Some(Value::Object(data)) => Ok(State::from_json(&data)),
        _ => {
            let backup = path.with_file_name(format!("{}.bak", file_name(path)));
            std::fs::rename(path, &backup)?;
            Ok(State::default())
        }
    }
}

/// Save atomically: a crash or a full disk midway leaves the previous file as it was.
pub fn save(state: &State, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(format!("{}.tmp", file_name(path)));
    let mut text = serde_json::to_string_pretty(&state.to_json()).map_err(io::Error::other)?;
    text.push('\n');
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "state.json".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    /// A time in September 2026. The 14th is a Monday.
    fn at(day: u32, hour: u32, minute: u32) -> Now {
        utc().with_ymd_and_hms(2026, 9, day, hour, minute, 0).unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn make_state(weekday: u32, hour: u32) -> State {
        State::with_steps(
            vec![Step::with_id("A", "", "", "a"), Step::with_id("B", "", "", "b")],
            Settings { weekday, hour, ..Settings::default() },
        )
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("update-check-test-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cycle_starts_on_most_recent_reminder_day_after_its_hour() {
        let state = make_state(0, 10);
        assert_eq!(state.current_cycle_start(&at(14, 10, 0)), date(2026, 9, 14));
        assert_eq!(state.current_cycle_start(&at(16, 8, 30)), date(2026, 9, 14));
        assert_eq!(state.current_cycle_start(&at(14, 9, 59)), date(2026, 9, 7));
        assert_eq!(state.next_reminder(&at(16, 0, 0)), date(2026, 9, 21));
    }

    #[test]
    fn new_cycle_clears_checks_but_keeps_history() {
        let mut state = make_state(0, 10);
        state.roll_over(&at(14, 11, 0));
        state.set_done("a", true, &at(14, 12, 0));
        state.set_done("b", true, &at(14, 12, 0));
        assert!(state.is_complete());

        assert!(!state.roll_over(&at(21, 9, 59)));
        assert!(state.roll_over(&at(21, 10, 0)));
        assert!(state.done.is_empty());
        assert!(!state.is_complete());
        assert_eq!(state.last_completed, Some(at(14, 12, 0)));
    }

    #[test]
    fn daylight_saving_change_does_not_restart_the_week() {
        let summer = FixedOffset::east_opt(2 * 3600).unwrap();
        let winter = FixedOffset::east_opt(3600).unwrap();
        let mut state = make_state(0, 10);
        state.roll_over(&summer.with_ymd_and_hms(2026, 10, 19, 11, 0, 0).unwrap());
        state.set_done("a", true, &summer.with_ymd_and_hms(2026, 10, 19, 11, 0, 0).unwrap());

        assert!(!state.roll_over(&winter.with_ymd_and_hms(2026, 10, 26, 9, 30, 0).unwrap()));
        assert_eq!(state.done, BTreeSet::from(["a".to_owned()]));
    }

    #[test]
    fn moving_reminder_to_earlier_day_keeps_progress() {
        let mut state = make_state(0, 10);
        state.roll_over(&at(14, 11, 0));
        state.set_done("a", true, &at(14, 11, 0));
        state.settings.weekday = 6;
        assert!(!state.roll_over(&at(15, 9, 0)));
        assert_eq!(state.done, BTreeSet::from(["a".to_owned()]));
    }

    #[test]
    fn unchecking_a_step_reopens_the_week() {
        let mut state = make_state(0, 10);
        state.roll_over(&at(14, 11, 0));
        state.set_done("a", true, &at(14, 11, 0));
        state.set_done("b", true, &at(14, 11, 0));
        state.set_done("b", false, &at(14, 12, 0));
        assert!(!state.is_complete());
    }

    #[test]
    fn reminds_at_most_once_a_day_until_complete() {
        let mut state = make_state(0, 10);
        let now = at(14, 10, 0);
        state.roll_over(&now);
        assert!(state.should_remind(&now));

        state.last_reminded = Some(now.date_naive());
        assert!(!state.should_remind(&at(14, 18, 0)));
        assert!(state.should_remind(&at(15, 10, 0)));

        state.set_done("a", true, &now);
        state.set_done("b", true, &now);
        assert!(!state.should_remind(&at(16, 10, 0)));
    }

    #[test]
    fn no_reminder_without_steps() {
        let mut state = State::with_steps(Vec::new(), Settings::default());
        state.roll_over(&at(14, 10, 0));
        assert!(!state.should_remind(&at(14, 10, 0)));
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = temp_dir("round-trip").join("nested").join("state.json");
        let mut state = make_state(4, 18);
        state.settings.appearance = "Dark".to_owned();
        state.settings.autostart = Some(false);
        state.roll_over(&at(18, 19, 0));
        state.set_done("a", true, &at(18, 19, 0));
        state.last_reminded = Some(date(2026, 9, 18));
        state.ignored_apps = ["Microsoft.BingWeather".to_owned(), "Microsoft OneDrive".to_owned()].into();

        save(&state, &path).unwrap();
        assert_eq!(load(&path).unwrap(), state);
    }

    #[test]
    fn loads_cycle_start_saved_as_timestamp() {
        let path = temp_dir("timestamp").join("state.json");
        save(&make_state(0, 10), &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("\"cycle_start\": null", "\"cycle_start\": \"2026-09-14T10:00:00\"")).unwrap();
        assert_eq!(load(&path).unwrap().cycle_start, Some(date(2026, 9, 14)));
    }

    #[test]
    fn missing_file_gives_default_checklist() {
        let state = load(&temp_dir("missing").join("state.json")).unwrap();
        assert_eq!(state, State::default());
        assert!(!state.steps.is_empty());
        // Not asked yet: the first start asks before setting anything up.
        assert_eq!(state.settings.autostart, None);
    }

    #[test]
    fn an_answer_about_the_reminder_is_kept() {
        let path = temp_dir("answer").join("state.json");
        let mut state = make_state(0, 10);
        state.settings.autostart = Some(true);
        save(&state, &path).unwrap();
        assert_eq!(load(&path).unwrap().settings.autostart, Some(true));
    }

    #[test]
    fn unreadable_file_is_backed_up() {
        let dir = temp_dir("unreadable");
        let path = dir.join("state.json");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path).unwrap(), State::default());
        assert_eq!(std::fs::read_to_string(dir.join("state.json.bak")).unwrap(), "{not json");
    }

    #[test]
    fn checklist_saved_before_a_new_default_step_gets_it_once() {
        let path = temp_dir("new-default").join("state.json");
        let mut state = State::default();
        state.steps.retain(|step| step.id != GRAPHICS_STEP);
        save(&state, &path).unwrap();
        let mut data: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        data.as_object_mut().unwrap().remove("offered_steps");
        std::fs::write(&path, data.to_string()).unwrap();

        let mut loaded = load(&path).unwrap();
        let ids: Vec<&str> = loaded.steps.iter().map(|step| step.id.as_str()).collect();
        let position = |id: &str| ids.iter().position(|own| *own == id).unwrap();
        assert_eq!(position(GRAPHICS_STEP), position("drivers") + 1);

        loaded.steps.retain(|step| step.id != GRAPHICS_STEP);
        save(&loaded, &path).unwrap();
        assert!(load(&path).unwrap().steps.iter().all(|step| step.id != GRAPHICS_STEP));
    }

    #[test]
    fn new_default_step_goes_after_the_last_earlier_default_step_or_at_the_end() {
        let offered: BTreeSet<String> = ORIGINAL_DEFAULT_STEPS.iter().map(|id| (*id).to_owned()).collect();
        let mut after_windows_update = State::with_steps(
            vec![Step::with_id("Windows Update", "", "", "windows-update"), Step::with_id("Mine", "", "", "mine")],
            Settings::default(),
        );
        let mut only_custom = State::with_steps(vec![Step::with_id("Mine", "", "", "mine")], Settings::default());
        for state in [&mut after_windows_update, &mut only_custom] {
            state.offered_steps = offered.clone();
            state.add_new_default_steps();
        }
        let ids = |state: &State| state.steps.iter().map(|step| step.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&after_windows_update), ["windows-update", GRAPHICS_STEP, STORAGE_STEP, APPS_STEP, "mine"]);
        assert_eq!(ids(&only_custom), ["mine", GRAPHICS_STEP, STORAGE_STEP, APPS_STEP]);
    }

    #[test]
    fn ticking_a_step_records_the_day_it_was_done() {
        let mut state = make_state(0, 10);
        state.roll_over(&at(14, 10, 0));
        state.set_done("a", true, &at(16, 9, 0));
        assert_eq!(state.last_done, BTreeMap::from([("a".to_owned(), date(2026, 9, 16))]));

        // Un-ticking is a correction to this week, not a claim that it was never done.
        state.set_done("a", false, &at(16, 9, 0));
        assert_eq!(state.last_done, BTreeMap::from([("a".to_owned(), date(2026, 9, 16))]));
    }

    #[test]
    fn last_done_and_the_streak_survive_a_round_trip() {
        let mut state = make_state(0, 10);
        state.roll_over(&at(14, 10, 0));
        state.set_done("a", true, &at(14, 11, 0));
        state.set_done("b", true, &at(14, 11, 0));
        let path = temp_dir("streak-trip").join("state.json");
        save(&state, &path).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.last_done,
            BTreeMap::from([("a".to_owned(), date(2026, 9, 14)), ("b".to_owned(), date(2026, 9, 14))])
        );
        assert_eq!(loaded.completed_weeks, [date(2026, 9, 14)]);
    }

    #[test]
    fn a_checklist_from_before_these_were_recorded_still_loads() {
        let path = temp_dir("old").join("state.json");
        std::fs::write(&path, r#"{"steps": [], "settings": {}}"#).unwrap();
        let state = load(&path).unwrap();
        assert!(state.last_done.is_empty());
        assert!(state.completed_weeks.is_empty());
    }

    #[test]
    fn a_checklist_the_python_version_saved_loads_and_saves_alike() {
        // As the first versions, written in Python, saved it.
        let text = r#"{
  "steps": [
    {"title": "Windows Update", "detail": "Install pending updates, then restart if asked.", "action": "ms-settings:windowsupdate", "id": "windows-update"},
    {"title": "Optional driver updates", "detail": "Review optional updates for drivers and firmware.", "action": "ms-settings:windowsupdate-optionalupdates", "id": "drivers"},
    {"title": "Graphics drivers", "detail": "", "action": "", "id": "graphics"},
    {"title": "Microsoft Store apps", "detail": "Open the Library and click “Get updates”.", "action": "ms-windows-store://downloadsandupdates", "id": "store"},
    {"title": "Desktop apps", "detail": "Upgrade everything winget manages.", "action": "winget upgrade --all", "id": "winget"}
  ],
  "settings": {"weekday": 0, "hour": 10, "appearance": "System", "autostart": true},
  "cycle_start": "2026-11-02",
  "done": ["drivers", "graphics", "store", "windows-update", "winget"],
  "last_completed": "2026-11-03T22:20:29.068829+01:00",
  "last_reminded": "2026-11-02",
  "offered_steps": ["backup", "defender", "drivers", "graphics", "storage", "store", "windows-update", "winget"],
  "last_done": {"backup": "2026-10-01", "winget": "2026-11-03"},
  "completed_weeks": ["2026-10-26", "2026-11-02"]
}"#;
        let path = temp_dir("python").join("state.json");
        std::fs::write(&path, text).unwrap();
        let state = load(&path).unwrap();
        let ids: Vec<&str> = state.steps.iter().map(|step| step.id.as_str()).collect();
        // The step added since comes after the last of the earlier default steps the checklist kept.
        assert_eq!(ids, ["windows-update", "drivers", "graphics", "store", "winget", APPS_STEP]);
        assert_eq!(state.steps[3].detail, "Open the Library and click “Get updates”.");
        assert_eq!(state.settings, Settings { weekday: 0, hour: 10, appearance: "System".to_owned(), autostart: Some(true) });
        assert_eq!(state.cycle_start, Some(date(2026, 11, 2)));
        assert_eq!(state.done.len(), 5);
        assert_eq!(state.last_completed.map(|time| time.to_rfc3339()).as_deref(), Some("2026-11-03T22:20:29.068829+01:00"));
        assert_eq!(state.last_reminded, Some(date(2026, 11, 2)));
        assert_eq!(state.last_done.get("winget"), Some(&date(2026, 11, 3)));
        assert_eq!(state.completed_weeks, [date(2026, 10, 26), date(2026, 11, 2)]);
        // Saved again, it reads back the same.
        save(&state, &path).unwrap();
        assert_eq!(load(&path).unwrap(), state);
    }

    #[test]
    fn odd_values_fall_back_to_their_defaults() {
        let path = temp_dir("odd").join("state.json");
        std::fs::write(
            &path,
            r#"{"steps": [{"title": "Kept"}, {"detail": "no title"}, 5], "settings": {"weekday": 9, "hour": "ten", "appearance": "Neon"}}"#,
        )
        .unwrap();
        let state = load(&path).unwrap();
        assert_eq!(state.steps[0].title, "Kept");
        assert_eq!(state.steps[0].id.len(), 8);
        assert_eq!(state.settings, Settings::default());
    }

    #[test]
    fn finishing_a_week_adds_it_to_the_streak() {
        let mut state = make_state(0, 10);
        state.completed_weeks = vec![date(2026, 8, 31), date(2026, 9, 7)];
        state.roll_over(&at(14, 10, 0));
        // This week isn't finished yet, which doesn't break the run behind it.
        assert_eq!(state.streak(&at(14, 10, 0)), 2);

        state.set_done("a", true, &at(14, 11, 0));
        state.set_done("b", true, &at(14, 11, 0));
        assert_eq!(state.streak(&at(14, 11, 0)), 3);
    }

    #[test]
    fn a_missed_week_ends_the_streak() {
        let mut state = make_state(0, 10);
        state.completed_weeks = vec![date(2026, 8, 24), date(2026, 9, 7)];
        state.roll_over(&at(14, 10, 0));
        assert_eq!(state.streak(&at(14, 10, 0)), 1);
    }

    #[test]
    fn un_ticking_takes_this_week_back_off_the_streak() {
        let mut state = make_state(0, 10);
        state.roll_over(&at(14, 10, 0));
        state.set_done("a", true, &at(14, 11, 0));
        state.set_done("b", true, &at(14, 11, 0));
        assert_eq!(state.completed_weeks, [date(2026, 9, 14)]);

        state.set_done("b", false, &at(14, 12, 0));
        assert!(state.completed_weeks.is_empty());
    }

    #[test]
    fn moving_the_reminder_day_keeps_the_streak() {
        let mut state = make_state(0, 10);
        // Three Mondays finished in a row.
        state.completed_weeks = vec![date(2026, 8, 31), date(2026, 9, 7), date(2026, 9, 14)];
        // The reminder moves to Wednesdays, so this week starts on the 16th and the weeks behind it no longer
        // line up with the days they were recorded on.
        state.settings.weekday = 2;
        assert_eq!(state.current_cycle_start(&at(17, 10, 0)), date(2026, 9, 16));
        assert_eq!(state.streak(&at(17, 10, 0)), 3);
    }

    #[test]
    fn last_completed_keeps_its_offset() {
        let mut state = make_state(0, 10);
        let when = FixedOffset::east_opt(3600).unwrap().from_utc_datetime(
            &NaiveDate::from_ymd_opt(2026, 9, 14).unwrap().and_time(NaiveTime::from_hms_opt(9, 30, 0).unwrap()),
        );
        state.last_completed = Some(when);
        let data = state.to_json();
        assert_eq!(data["last_completed"], "2026-09-14T10:30:00.000000+01:00");
        let loaded = State::from_json(data.as_object().unwrap());
        assert_eq!(loaded.last_completed.unwrap().offset(), when.offset());
    }
}
