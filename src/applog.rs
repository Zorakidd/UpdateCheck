//! The app's log: warnings and unexpected errors, for when something didn't work. It lives beside the
//! checklist as update-check.log and is kept to about a megabyte, with the previous log as update-check.log.1.

use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const LIMIT: u64 = 1024 * 1024;

static WRITER: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn log_file() -> PathBuf {
    crate::state::data_file().with_file_name("update-check.log")
}

#[derive(Clone, Copy)]
enum Level {
    Info,
    Warning,
    Error,
}

pub fn info(target: &str, message: impl Display) {
    write(Level::Info, target, message);
}

pub fn warning(target: &str, message: impl Display) {
    write(Level::Warning, target, message);
}

pub fn error(target: &str, message: impl Display) {
    write(Level::Error, target, message);
}

fn line(level: Level, target: &str, message: &dyn Display) -> String {
    let level = match level {
        Level::Info => "INFO",
        Level::Warning => "WARNING",
        Level::Error => "ERROR",
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f");
    format!("{now} {level} update_check.{target}: {message}\n")
}

/// Opened on the first message, so a start without anything to report doesn't touch the disk.
fn write(level: Level, target: &str, message: impl Display) {
    let line = line(level, target, &message);
    if cfg!(test) {
        eprint!("{line}");
        return;
    }
    let Ok(mut writer) = WRITER.lock() else { return };
    if writer.is_none() {
        let path = log_file();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > LIMIT) {
            let _ = std::fs::rename(&path, path.with_file_name("update-check.log.1"));
        }
        *writer = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
    }
    if let Some(file) = writer.as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Log panics, which are bugs, with where they happened. The app shows its own message afterwards.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".to_owned());
        let place = info.location().map(|place| format!(" at {}:{}", place.file(), place.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        error("panic", format!("Unexpected error in thread {thread}{place}: {message}"));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_read_like_the_first_versions_log() {
        let text = line(Level::Warning, "updater", &"Update failed: offline");
        let (time, rest) = text.split_at(23);
        assert!(chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%d %H:%M:%S,%3f").is_ok(), "{time}");
        assert_eq!(rest, " WARNING update_check.updater: Update failed: offline\n");
    }
}
