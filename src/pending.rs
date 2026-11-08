//! How much is actually waiting: pending Windows updates, driver updates and winget package upgrades.
//!
//! Neither tool offers a machine-readable count. winget prints a table in whatever language Windows is set to,
//! and Windows Update answers only through COM, so both are read defensively: anything that doesn't look like
//! an answer is reported as unknown rather than guessed at. Both cost seconds and a network round trip, so they
//! run in background threads and the checklist shows whatever has come back.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, TimeZone, Utc};

use crate::applog;
use crate::system;

pub const WINDOWS_UPDATE: &str = "windows-update";
pub const DRIVERS: &str = "drivers";
pub const WINGET: &str = "winget";

/// Windows Update's search regularly takes 15 seconds; winget has to reach its sources.
const WINDOWS_UPDATE_TIMEOUT: Duration = Duration::from_secs(120);
const WINGET_TIMEOUT: Duration = Duration::from_secs(90);
/// Counts this large mean the output was misread, not that the PC is that far behind.
const IMPLAUSIBLE: usize = 1000;
/// How much shorter than the table a row may end: by as much as its source's name, such as "winget", is
/// shorter than the last column, which is as wide as the longest source name or the column's translated heading.
const SOURCE_SLACK: usize = 12;
/// How long a failed lookup waits before it is tried again, for example once the PC is back online.
const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Checking,
    NothingPending,
    Pending(usize),
    CheckFailed,
}

/// How much a lookup found waiting, what it is where that can be told, and, for Windows Update, the day it
/// last installed updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub count: usize,
    pub last_installed: Option<NaiveDate>,
    /// What is waiting, such as "Mozilla Firefox 145.0.1 → 146.0", if the lookup can name it.
    pub items: Vec<String>,
}

impl Found {
    pub fn count(count: usize) -> Self {
        Found { count, last_installed: None, items: Vec::new() }
    }
}

/// Lines as Python's `str.splitlines` splits them, which is what the first versions read winget with.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, char)) = chars.next() {
        let breaks = matches!(
            char,
            '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        );
        if !breaks {
            continue;
        }
        lines.push(&text[start..index]);
        let mut end = index + char.len_utf8();
        if char == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            chars.next();
            end += 1;
        }
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Whether a console gives the character two columns, as it does in Chinese, Japanese and Korean.
fn is_wide(char: char) -> bool {
    matches!(char as u32,
        0x1100..=0x115F | 0x231A..=0x231B | 0x2329..=0x232A | 0x23E9..=0x23EC | 0x23F0 | 0x23F3
        | 0x25FD..=0x25FE | 0x2614..=0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA..=0x26AB
        | 0x26BD..=0x26BE | 0x26C4..=0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2..=0x26F3 | 0x26F5 | 0x26FA
        | 0x26FD | 0x2705 | 0x270A..=0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757
        | 0x2795..=0x2797 | 0x27B0 | 0x27BF | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55
        | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA000..=0xA4CF
        | 0xA960..=0xA97F | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F004 | 0x1F0CF | 0x1F18E | 0x1F191..=0x1F19A
        | 0x1F200..=0x1F202 | 0x1F210..=0x1F23B | 0x1F240..=0x1F248 | 0x1F250..=0x1F251
        | 0x1F300..=0x1F64F | 0x1F680..=0x1F6FF | 0x1F7E0..=0x1F7EB | 0x1F900..=0x1F9FF | 0x1FA70..=0x1FAFF
        | 0x20000..=0x2FFFD | 0x30000..=0x3FFFD)
}

/// How many columns a console gives the text.
fn width(text: &str) -> usize {
    text.chars().map(|char| if is_wide(char) { 2 } else { 1 }).sum()
}

/// Count the rows of winget's upgrade table.
///
/// The table is found by shape rather than by its words, which are translated: a rule of dashes as wide as the
/// table, then one row per package, then a summary sentence. winget pads every column but the last, so a
/// package's row runs as wide as the rule, or nearly: only the source's name in the last column can fall short
/// of it. The sentence underneath is shorter, or ends in a full stop rather than a source's name.
///
/// Spaces don't tell them apart: where a package has the widest entry in every column, its row has nothing but
/// single spaces, just like a sentence.
///
/// Without a table, only the exit code tells "nothing to upgrade" from an error: winget explains both in a
/// single translated sentence.
pub fn count_winget_table(output: &str, exit_code: i64) -> Option<usize> {
    match find_table(output) {
        Some(table) => (table.rows.len() < IMPLAUSIBLE).then_some(table.rows.len()),
        // No rule at all means winget said something else: that everything is up to date if it exited
        // cleanly, or why it failed.
        None => (!split_lines(output).is_empty() && exit_code == 0).then_some(0),
    }
}

/// The first table in winget's output: the line above its rule, which holds the column headings, and its rows.
struct Table<'a> {
    header: Option<&'a str>,
    rows: Vec<&'a str>,
}

fn find_table(output: &str) -> Option<Table<'_>> {
    let lines = split_lines(output);
    for (index, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if stripped.chars().count() > 10 && stripped.chars().all(|char| char == '-') {
            let rule = width(line.trim_end());
            let mut rows = Vec::new();
            for row in &lines[index + 1..] {
                let text = row.trim_end();
                let row_width = width(text);
                let fits = rule.saturating_sub(SOURCE_SLACK) <= row_width && row_width <= rule + 1;
                if !(fits && text.chars().last().is_some_and(char::is_alphanumeric)) {
                    break;
                }
                rows.push(*row);
            }
            let header = index.checked_sub(1).map(|above| lines[above]);
            return Some(Table { header, rows });
        }
    }
    None
}

/// The packages in winget's upgrade table, as "Mozilla Firefox 145.0.1 → 146.0".
///
/// The headings are translated, but they stand where their columns begin: the name runs up to the second
/// heading, the installed version from the third to the fourth and the new one from the fourth to the fifth.
/// A table whose headings can't be told apart gives no names, and the count alone is shown.
pub fn winget_table_items(output: &str) -> Vec<String> {
    let Some(Table { header: Some(header), rows }) = find_table(output) else { return Vec::new() };
    let starts = word_starts(header);
    if starts.len() < 4 || rows.len() >= IMPLAUSIBLE {
        return Vec::new();
    }
    let end = starts.get(4).copied().unwrap_or(usize::MAX);
    rows.iter()
        .map(|row| {
            let name = columns(row, 0, starts[1]);
            let installed = columns(row, starts[2], starts[3]);
            let available = columns(row, starts[3], end);
            // winget writes "< 2.0" where it can't tell which older version is installed.
            if available.is_empty() {
                name
            } else if installed.is_empty() || installed.starts_with('<') {
                format!("{name} → {available}")
            } else {
                format!("{name} {installed} → {available}")
            }
        })
        .filter(|item| !item.is_empty())
        .collect()
}

/// The columns at which the words of a line begin.
fn word_starts(line: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut column = 0;
    let mut after_space = true;
    for char in line.chars() {
        let space = char.is_whitespace();
        if !space && after_space {
            starts.push(column);
        }
        after_space = space;
        column += if is_wide(char) { 2 } else { 1 };
    }
    starts
}

/// The text of a line from column `from` up to column `to`, trimmed.
fn columns(line: &str, from: usize, to: usize) -> String {
    let mut column = 0;
    let mut text = String::new();
    for char in line.chars() {
        if (from..to).contains(&column) {
            text.push(char);
        }
        column += if is_wide(char) { 2 } else { 1 };
    }
    text.trim().to_owned()
}

/// How many packages winget can upgrade and which, or None if its answer couldn't be read.
pub fn winget_upgrades() -> Result<Option<Found>, String> {
    let output = system::run_hidden(&system::winget(), &["upgrade", "--disable-interactivity"], WINGET_TIMEOUT)
        .map_err(|error| format!("winget couldn't be run: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    // winget's own error codes, such as 0x8A15000F, are negative as the 32-bit numbers they are.
    let code = output.code.map_or(-1, |code| i64::from(code as u32));
    Ok(count_winget_table(&text, code).map(|count| {
        let mut items = winget_table_items(&text);
        // Names only when they match the count, so the list never says something different from the number.
        if items.len() != count {
            items.clear();
        }
        Found { count, last_installed: None, items }
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsUpdateSearch {
    pub software: usize,
    pub drivers: usize,
    pub last_installed: Option<NaiveDate>,
}

/// Read the PowerShell search's answer, such as "3 1 2026-11-03", or None if it isn't one.
pub fn parse_search(output: &str) -> Option<WindowsUpdateSearch> {
    let parts: Vec<&str> = output.split_whitespace().collect();
    let [software, drivers, day] = parts[..] else { return None };
    let number = |text: &str| -> Option<usize> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse().ok().filter(|count| *count < IMPLAUSIBLE)
    };
    Some(WindowsUpdateSearch {
        software: number(software)?,
        drivers: number(drivers)?,
        last_installed: NaiveDate::parse_from_str(day, "%Y-%m-%d").ok(),
    })
}

/// One search for both Windows Update steps: type 1 is software, Windows itself included, and type 2 drivers
/// and firmware. The day updates were last installed comes along, which tells how current Windows is.
const SEARCH_SCRIPT: &str = "$ErrorActionPreference='Stop';\
$updates = (New-Object -ComObject Microsoft.Update.Session).CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0').Updates;\
$software = @($updates | Where-Object { $_.Type -eq 1 }).Count;\
$drivers = @($updates | Where-Object { $_.Type -eq 2 }).Count;\
$installed = (New-Object -ComObject Microsoft.Update.AutoUpdate).Results.LastInstallationSuccessDate;\
$day = if ($installed -and $installed.Year -gt 2000) { $installed.ToString('yyyy-MM-dd') } else { '-' };\
($software, $drivers, $day) -join ' '";

/// The day an OLE date falls on, in local time: days since the end of 1899, in UTC as Windows Update keeps it.
fn ole_date(value: f64) -> Option<NaiveDate> {
    if !value.is_finite() || value < 1.0 {
        return None;
    }
    let seconds = ((value - 25569.0) * 86400.0).round() as i64;
    let when = Utc.timestamp_opt(seconds, 0).single()?;
    let day = when.with_timezone(&Local).date_naive();
    (day > NaiveDate::from_ymd_opt(2000, 12, 31)?).then_some(day)
}

/// Ask the Windows Update Agent directly, as a standard user can.
fn search_with_com() -> windows::core::Result<WindowsUpdateSearch> {
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
    use windows::Win32::System::UpdateAgent::{AutomaticUpdates, IAutomaticUpdates2, IUpdateSession, UpdateSession};
    use windows::Win32::System::Variant::VT_DATE;
    use windows::core::BSTR;

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let result = (|| {
            let session: IUpdateSession = CoCreateInstance(&UpdateSession, None, CLSCTX_ALL)?;
            let updates = session.CreateUpdateSearcher()?.Search(&BSTR::from("IsInstalled=0 and IsHidden=0"))?.Updates()?;
            let (mut software, mut drivers) = (0, 0);
            for index in 0..updates.Count()? {
                match updates.get_Item(index)?.Type()?.0 {
                    1 => software += 1,
                    2 => drivers += 1,
                    _ => {}
                }
            }
            let last_installed = (|| -> windows::core::Result<Option<NaiveDate>> {
                let automatic: IAutomaticUpdates2 = CoCreateInstance(&AutomaticUpdates, None, CLSCTX_ALL)?;
                let value = automatic.Results()?.LastInstallationSuccessDate()?;
                let inner = &value.Anonymous.Anonymous;
                Ok(if inner.vt == VT_DATE { ole_date(inner.Anonymous.date) } else { None })
            })()
            .ok()
            .flatten();
            Ok(WindowsUpdateSearch { software, drivers, last_installed })
        })();
        CoUninitialize();
        result
    }
}

/// The same search through PowerShell, should Windows Update's COM interface refuse this process.
fn search_with_powershell() -> Option<WindowsUpdateSearch> {
    let output = system::run_hidden(
        &system::powershell(),
        &["-NoProfile", "-NonInteractive", "-Command", SEARCH_SCRIPT],
        WINDOWS_UPDATE_TIMEOUT,
    )
    .ok()?;
    if output.code != Some(0) {
        return None;
    }
    parse_search(&String::from_utf8_lossy(&output.stdout))
}

/// What Windows Update has waiting. A search that hangs is given up on after two minutes.
pub fn search_windows_update() -> Option<WindowsUpdateSearch> {
    let (sender, receiver) = channel();
    std::thread::Builder::new()
        .name("windows-update".into())
        .spawn(move || {
            let result = match search_with_com() {
                Ok(result) => Some(result),
                Err(error) => {
                    applog::info("pending", format!("Windows Update's COM search failed ({error}); trying PowerShell"));
                    search_with_powershell()
                }
            };
            let _ = sender.send(result);
        })
        .ok()?;
    receiver.recv_timeout(WINDOWS_UPDATE_TIMEOUT).ok().flatten()
}

type Search = Box<dyn Fn() -> Option<WindowsUpdateSearch> + Send + Sync>;

/// Windows Update searched once for every lookup that asks within a minute. A search takes a while, and a
/// second one started alongside would only wait for the first.
pub struct SharedSearch {
    search: Search,
    fresh_for: Duration,
    /// Raised by `forget`, so a search cached or still running from before then no longer counts as fresh.
    generation: AtomicU64,
    last: Mutex<Option<(Instant, u64, Option<WindowsUpdateSearch>)>>,
}

impl SharedSearch {
    pub fn new(search: Search) -> Self {
        SharedSearch { search, fresh_for: Duration::from_secs(60), generation: AtomicU64::new(0), last: Mutex::new(None) }
    }

    pub fn get(&self) -> Option<WindowsUpdateSearch> {
        let mut last = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some((when, of, result)) = *last
            && of == generation
            && when.elapsed() < self.fresh_for
        {
            return result;
        }
        let result = (self.search)();
        *last = Some((Instant::now(), generation, result));
        result
    }

    /// Forget the last search, so the next lookup asks Windows Update again, such as after the user installed
    /// updates. Doesn't wait for a search that is running.
    pub fn forget(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

pub type LookupFn = Arc<dyn Fn() -> Result<Option<Found>, String> + Send + Sync>;

/// The lookups for the steps whose counts can be told, by step id, sharing one Windows Update search with
/// whatever else asks, such as the PC info tab.
pub fn standard_lookups(search: Arc<SharedSearch>) -> HashMap<&'static str, LookupFn> {
    let mut lookups: HashMap<&'static str, LookupFn> = HashMap::new();
    lookups.insert(WINGET, Arc::new(winget_upgrades));
    let shared = search.clone();
    lookups.insert(
        WINDOWS_UPDATE,
        Arc::new(move || {
            Ok(shared.get().map(|found| Found { last_installed: found.last_installed, ..Found::count(found.software) }))
        }),
    );
    lookups.insert(DRIVERS, Arc::new(move || Ok(search.get().map(|found| Found::count(found.drivers)))));
    lookups
}

/// '12 upgrades waiting', in the words that suit the step.
pub fn describe(step_id: &str, count: usize) -> String {
    let (one, many) = match step_id {
        WINGET => ("upgrade", "upgrades"),
        WINDOWS_UPDATE => ("update", "updates"),
        DRIVERS => ("driver update", "driver updates"),
        _ => ("item", "items"),
    };
    format!("{count} {} waiting", if count == 1 { one } else { many })
}

#[derive(Clone)]
enum Lookup {
    Found(Option<Found>),
    Failed,
}

/// Counts of what each step has waiting, looked up in background threads.
///
/// `refresh()` starts the lookups for the steps a checklist actually has; `collect()`, called from the
/// window's thread, takes in the finished ones.
pub struct PendingChecks {
    lookups: HashMap<&'static str, LookupFn>,
    results: HashMap<String, Lookup>,
    failed_at: HashMap<String, Instant>,
    running: HashSet<String>,
    sender: Sender<(String, Lookup)>,
    receiver: Receiver<(String, Lookup)>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl PendingChecks {
    pub fn new(lookups: HashMap<&'static str, LookupFn>, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (sender, receiver) = channel();
        PendingChecks {
            lookups,
            results: HashMap::new(),
            failed_at: HashMap::new(),
            running: HashSet::new(),
            sender,
            receiver,
            notify,
        }
    }

    #[cfg(test)]
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
    }

    pub fn handles(&self, step_id: &str) -> bool {
        self.lookups.contains_key(step_id)
    }

    /// Look up the counts for the steps in `step_ids` that haven't been counted, whose lookup failed a while
    /// ago, or all of them with `recheck`. Returns true if anything shown changed.
    pub fn refresh<'a>(&mut self, step_ids: impl IntoIterator<Item = &'a str>, recheck: bool) -> bool {
        let mut changed = false;
        for step_id in step_ids {
            let Some(lookup) = self.lookups.get(step_id).cloned() else { continue };
            if self.running.contains(step_id) {
                continue;
            }
            let retry = matches!(self.results.get(step_id), Some(Lookup::Failed))
                && self.failed_at.get(step_id).is_some_and(|when| when.elapsed() >= RETRY_AFTER);
            if !(recheck || !self.results.contains_key(step_id) || retry) {
                continue;
            }
            self.results.remove(step_id);
            self.running.insert(step_id.to_owned());
            let sender = self.sender.clone();
            let notify = self.notify.clone();
            let id = step_id.to_owned();
            let started = std::thread::Builder::new().name(format!("pending-{id}")).spawn(move || {
                let result = match lookup() {
                    Ok(found) => Lookup::Found(found),
                    Err(error) => {
                        // These are other people's command line tools: whatever goes wrong, the count is unknown.
                        applog::warning("pending", format!("Couldn't count what {id} has waiting: {error}"));
                        Lookup::Failed
                    }
                };
                let _ = sender.send((id, result));
                notify();
            });
            if started.is_err() {
                self.running.remove(step_id);
                self.results.insert(step_id.to_owned(), Lookup::Failed);
                self.failed_at.insert(step_id.to_owned(), Instant::now());
            }
            changed = true;
        }
        changed
    }

    /// Take in finished lookups. Returns true if there were any.
    pub fn collect(&mut self) -> bool {
        let mut collected = false;
        while let Ok((step_id, result)) = self.receiver.try_recv() {
            self.running.remove(&step_id);
            if matches!(result, Lookup::Failed) {
                self.failed_at.insert(step_id.clone(), Instant::now());
            }
            self.results.insert(step_id, result);
            collected = true;
        }
        collected
    }

    pub fn status(&self, step_id: &str) -> Status {
        if !self.lookups.contains_key(step_id) {
            return Status::CheckFailed;
        }
        match self.results.get(step_id) {
            None => Status::Checking,
            Some(Lookup::Failed | Lookup::Found(None)) => Status::CheckFailed,
            Some(Lookup::Found(Some(found))) if found.count == 0 => Status::NothingPending,
            Some(Lookup::Found(Some(found))) => Status::Pending(found.count),
        }
    }

    /// The day Windows last installed updates, once the step's lookup has found out.
    pub fn last_installed(&self, step_id: &str) -> Option<NaiveDate> {
        match self.results.get(step_id) {
            Some(Lookup::Found(Some(found))) => found.last_installed,
            _ => None,
        }
    }

    /// What the step has waiting, by name, where its lookup can tell.
    pub fn items(&self, step_id: &str) -> &[String] {
        match self.results.get(step_id) {
            Some(Lookup::Found(Some(found))) => &found.items,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "Name                                ID                         Version Verfügbar Quelle\n\
---------------------------------------------------------------------------------------\n\
Mozilla Firefox                     Mozilla.Firefox            145.0.1 146.0     winget\n\
GitHub CLI                          GitHub.cli                 2.100.0 2.101.0   winget\n\
Microsoft GameInput                 Microsoft.GameInput        3.4.218 3.5.270   winget\n\
Microsoft Visual Studio Code (User) Microsoft.VisualStudioCode 1.137.0 1.138.0   winget\n\
4 Aktualisierungen verfügbar.\n";

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn checks(step: &'static str, lookup: impl Fn() -> Result<Option<Found>, String> + Send + Sync + 'static) -> PendingChecks {
        let mut lookups: HashMap<&'static str, LookupFn> = HashMap::new();
        lookups.insert(step, Arc::new(lookup));
        PendingChecks::new(lookups, Arc::new(|| {}))
    }

    fn wait_for(checks: &mut PendingChecks) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while checks.busy() && Instant::now() < deadline {
            checks.collect();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn rows_are_counted_whatever_language_winget_speaks() {
        assert_eq!(count_winget_table(TABLE, 0), Some(4));
    }

    #[test]
    fn a_row_with_the_widest_entry_in_every_column_still_counts() {
        // What winget said on 2026-11-03: Audacity fills every column, so its row has only single spaces.
        let output = "Name         ID                Version         Verfügbar       Quelle\n\
---------------------------------------------------------------------\n\
Audacity 4.0 Audacity.Audacity 4.0.1.262721353 4.0.2.263060742 winget\n\
Obsidian     Obsidian.Obsidian 1.14.5          1.15.0          winget\n\
2 Aktualisierungen verfügbar.\n";
        assert_eq!(count_winget_table(output, 0), Some(2));
    }

    #[test]
    fn rows_from_different_sources_count() {
        let output = "Name      Id                Version   Available Source\n\
------------------------------------------------------\n\
Notepad++ Notepad++.Notepad++ 8.6     8.7       winget\n\
Spotify   9NCBCSZSJRSB      1.2.40    1.2.45    msstore\n\
2 upgrades available.\n";
        assert_eq!(count_winget_table(output, 0), Some(2));
    }

    #[test]
    fn wide_characters_take_two_columns() {
        // A long Japanese name: 15 characters taking 30 columns, so the row is as wide as the rule in the
        // console although it has far fewer characters.
        let name = "日本語の長いアプリケーション名";
        let row = format!("{name}{}winget", " ".repeat(24));
        let output = format!("名前\n{}\n{row}\n1 個のアップグレードが利用可能です。\n", "-".repeat(60));
        assert!(row.chars().count() < 60 - 12);
        assert_eq!(count_winget_table(&output, 0), Some(1));
    }

    #[test]
    fn a_wordier_summary_is_still_not_counted_as_a_row() {
        let wordy = TABLE.replace(
            "4 Aktualisierungen verfügbar.",
            "4 Aktualisierungen sind verfügbar und können jetzt installiert werden.",
        );
        assert_eq!(count_winget_table(&wordy, 0), Some(4));
    }

    #[test]
    fn no_table_means_nothing_to_upgrade() {
        assert_eq!(count_winget_table("Keine installierten Pakete gefunden.\n", 0), Some(0));
    }

    #[test]
    fn no_output_at_all_is_not_an_answer() {
        assert_eq!(count_winget_table("", 0), None);
    }

    #[test]
    fn a_failing_winget_is_not_taken_for_nothing_to_upgrade() {
        // What winget said, and the code it exited with, when asked to search a source that doesn't exist.
        let output = "Keine Quellen entsprechen dem angegebenen Wert: gibtsnicht\nDie konfigurierten Quellen sind:\n  winget\n";
        assert_eq!(count_winget_table(output, 0x8A15_0012), None);
    }

    #[test]
    fn the_rows_of_a_table_count_whatever_the_exit_code() {
        // Packages winget did list are waiting, whatever else went wrong.
        assert_eq!(count_winget_table(TABLE, 0x8A15_000F), Some(4));
    }

    #[test]
    fn an_absurd_number_of_rows_is_not_an_answer() {
        let rows: String = (0..1200).map(|index| format!("Package{index}   Id{index}   1.0   2.0   winget\n")).collect();
        assert_eq!(count_winget_table(&format!("{}\n{rows}", "-".repeat(40)), 0), None);
    }

    #[test]
    fn winget_progress_spinners_do_not_hide_the_table() {
        let output = format!("\r-\r\\\r|\r/\r{TABLE}");
        assert_eq!(count_winget_table(&output, 0), Some(4));
        assert_eq!(count_winget_table(&TABLE.replace('\n', "\r\n"), 0), Some(4));
    }

    #[test]
    fn winget_names_the_packages_it_can_upgrade() {
        let items = winget_table_items(&format!("\r-\r\\\r{TABLE}"));
        assert_eq!(items.len(), 4);
        assert_eq!(items[0], "Mozilla Firefox 145.0.1 → 146.0");
        assert_eq!(items[3], "Microsoft Visual Studio Code (User) 1.137.0 → 1.138.0");
        let unsure = "Name            ID              Version          Available      Source
            ---------------------------------------------------------------------
            Ubisoft Connect Ubisoft.Connect < 175.1.0.13416  175.1.0.13416  winget
";
        let unsure: String = unsure.lines().map(|line| format!("{}
", line.trim_start())).collect();
        assert_eq!(winget_table_items(&unsure), ["Ubisoft Connect → 175.1.0.13416"]);
        // Without headings to go by, there are no names, only the count.
        assert!(winget_table_items(&TABLE.lines().skip(1).collect::<Vec<_>>().join("\n")).is_empty());
    }

    #[test]
    fn counts_are_described_in_the_words_of_the_step() {
        assert_eq!(describe("winget", 1), "1 upgrade waiting");
        assert_eq!(describe("winget", 4), "4 upgrades waiting");
        assert_eq!(describe("windows-update", 1), "1 update waiting");
        assert_eq!(describe("windows-update", 0), "0 updates waiting");
        assert_eq!(describe("drivers", 2), "2 driver updates waiting");
    }

    #[test]
    fn the_windows_update_search_is_read_strictly() {
        assert_eq!(
            parse_search("3 1 2026-11-03\n"),
            Some(WindowsUpdateSearch { software: 3, drivers: 1, last_installed: Some(date(2026, 11, 3)) })
        );
        // Windows can't tell when it last installed updates.
        assert_eq!(parse_search("0 0 -"), Some(WindowsUpdateSearch { software: 0, drivers: 0, last_installed: None }));
        for output in ["", "Zugriff verweigert", "3 1", "5000 0 2026-11-03", "-1 0 -"] {
            assert_eq!(parse_search(output), None, "{output:?}");
        }
    }

    #[test]
    fn ole_dates_are_read_as_days() {
        // 2026-11-03 12:00 UTC.
        let near_utc = Local::now().offset().local_minus_utc().abs() < 12 * 3600;
        assert_eq!(ole_date(46329.5), near_utc.then_some(date(2026, 11, 3)));
        assert_eq!(ole_date(0.0), None);
        assert_eq!(ole_date(f64::NAN), None);
    }

    #[test]
    fn both_windows_update_steps_share_one_search() {
        let searches = Arc::new(Mutex::new(0));
        let counter = searches.clone();
        let shared = Arc::new(SharedSearch::new(Box::new(move || {
            *counter.lock().unwrap() += 1;
            std::thread::sleep(Duration::from_millis(100));
            Some(WindowsUpdateSearch { software: 2, drivers: 1, last_installed: Some(date(2026, 11, 3)) })
        })));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let shared = shared.clone();
                std::thread::spawn(move || shared.get())
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|thread| thread.join().unwrap()).collect();
        assert_eq!(*searches.lock().unwrap(), 1);
        assert!(results.iter().all(|result| result.is_some_and(|found| found.software == 2)));
    }

    #[test]
    fn a_while_later_windows_update_is_searched_again() {
        let searches = Arc::new(Mutex::new(0));
        let counter = searches.clone();
        let mut shared = SharedSearch::new(Box::new(move || {
            *counter.lock().unwrap() += 1;
            None
        }));
        shared.get();
        shared.fresh_for = Duration::ZERO;
        shared.get();
        assert_eq!(*searches.lock().unwrap(), 2);
    }

    #[test]
    fn a_forgotten_search_is_made_again_at_once() {
        let searches = Arc::new(Mutex::new(0));
        let counter = searches.clone();
        let shared = SharedSearch::new(Box::new(move || {
            *counter.lock().unwrap() += 1;
            None
        }));
        shared.get();
        shared.get();
        shared.forget();
        shared.get();
        assert_eq!(*searches.lock().unwrap(), 2);
    }

    #[test]
    fn the_day_windows_last_installed_updates_comes_with_the_count() {
        let mut checks =
            checks(WINDOWS_UPDATE, || Ok(Some(Found { last_installed: Some(date(2026, 11, 3)), ..Found::count(2) })));
        assert_eq!(checks.last_installed(WINDOWS_UPDATE), None);
        checks.refresh([WINDOWS_UPDATE], false);
        wait_for(&mut checks);
        assert_eq!(checks.status(WINDOWS_UPDATE), Status::Pending(2));
        assert_eq!(checks.last_installed(WINDOWS_UPDATE), Some(date(2026, 11, 3)));
    }

    #[test]
    fn a_count_is_looked_up_in_the_background() {
        let mut checks = checks(WINGET, || Ok(Some(Found::count(4))));
        assert!(checks.refresh([WINGET], false));
        assert_eq!(checks.status(WINGET), Status::Checking);
        wait_for(&mut checks);
        assert_eq!(checks.status(WINGET), Status::Pending(4));
        // Already counted: nothing to do until a recheck.
        assert!(!checks.refresh([WINGET], false));
        assert!(checks.refresh([WINGET], true));
    }

    #[test]
    fn nothing_waiting_is_its_own_answer() {
        let mut checks = checks(WINGET, || Ok(Some(Found::count(0))));
        checks.refresh([WINGET], false);
        wait_for(&mut checks);
        assert_eq!(checks.status(WINGET), Status::NothingPending);
    }

    #[test]
    fn an_unreadable_answer_counts_as_a_failed_check() {
        let mut checks = checks(WINGET, || Ok(None));
        checks.refresh([WINGET], false);
        wait_for(&mut checks);
        assert_eq!(checks.status(WINGET), Status::CheckFailed);
    }

    #[test]
    fn a_tool_that_blows_up_counts_as_a_failed_check() {
        let mut checks = checks(WINGET, || Err("winget is not installed".to_owned()));
        checks.refresh([WINGET], false);
        wait_for(&mut checks);
        assert_eq!(checks.status(WINGET), Status::CheckFailed);
    }

    #[test]
    fn a_step_the_checklist_no_longer_has_is_not_looked_up() {
        let mut checks = checks(WINGET, || Ok(Some(Found::count(1))));
        assert!(!checks.refresh(["defender"], false));
        assert!(!checks.busy());
        assert_eq!(checks.status("defender"), Status::CheckFailed);
    }

    #[test]
    fn failed_lookup_is_retried_after_a_while() {
        let mut checks = checks(WINGET, || Err("offline".to_owned()));
        checks.refresh([WINGET], false);
        wait_for(&mut checks);
        assert!(!checks.refresh([WINGET], false));
        checks.failed_at.insert(WINGET.to_owned(), Instant::now() - RETRY_AFTER);
        assert!(checks.refresh([WINGET], false));
    }
}
