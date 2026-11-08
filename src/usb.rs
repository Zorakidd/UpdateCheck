//! Resetting a USB drive or memory card: erasing it and setting it up again as one empty drive, for when it
//! shows the wrong size, won't format or still holds an old Windows installer.
//!
//! Erasing the wrong drive can't be undone. So only USB drives and memory cards are offered, never one that
//! Windows or this app runs from, and the script that erases checks all of that again itself, as
//! administrator, right before it does.

use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::applog;
use crate::hardware::{self, BUS_MMC, BUS_SD, BUS_USB, Disk};
use crate::system;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSystem {
    ExFat,
    Ntfs,
    Fat32,
}

pub const FILE_SYSTEMS: [FileSystem; 3] = [FileSystem::ExFat, FileSystem::Ntfs, FileSystem::Fat32];

/// Windows formats FAT32 only up to 32 GB.
const FAT32_LIMIT: u64 = 32 * 1024 * 1024 * 1024;

/// Drives bigger than this are more often backups than sticks.
const LARGE: u64 = 300 * 1000 * 1000 * 1000;

/// Above this, MBR can't address the whole drive.
const MBR_LIMIT: u64 = 2 * 1024 * 1024 * 1024 * 1024;

impl FileSystem {
    pub fn name(self) -> &'static str {
        match self {
            FileSystem::ExFat => "exFAT",
            FileSystem::Ntfs => "NTFS",
            FileSystem::Fat32 => "FAT32",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            FileSystem::ExFat => "Works with Windows, Macs, cameras and TVs. The usual choice.",
            FileSystem::Ntfs => "For Windows only. Good for backups.",
            FileSystem::Fat32 => "For older devices such as car radios. Files up to 4 GB.",
        }
    }

    /// Whether Windows can format a drive of this size with it.
    pub fn fits(self, size: u64) -> bool {
        self != FileSystem::Fat32 || size <= FAT32_LIMIT
    }

    fn label_limit(self) -> usize {
        if self == FileSystem::Ntfs { 32 } else { 11 }
    }
}

/// A name the file system takes: letters, digits, spaces, dashes and underscores, as long as it allows, and
/// in capitals for FAT32, which stores them that way.
pub fn clean_label(text: &str, file_system: FileSystem) -> String {
    let kept: String = text.chars().filter(|char| char.is_ascii_alphanumeric() || matches!(char, ' ' | '_' | '-')).collect();
    let words = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let label: String = words.chars().take(file_system.label_limit()).collect();
    let label = label.trim_end().to_owned();
    if file_system == FileSystem::Fat32 { label.to_ascii_uppercase() } else { label }
}

/// The name the drive gets unless one is typed: the one it has, or "USB".
pub fn default_label(disk: &Disk, file_system: FileSystem) -> String {
    let current = disk.volumes.iter().map(|volume| clean_label(&volume.label, file_system)).find(|label| !label.is_empty());
    current.unwrap_or_else(|| "USB".to_owned())
}

/// The drive letter a path is on, such as "D:".
fn drive_of(path: &Path) -> Option<String> {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => Some(format!("{}:", letter.to_ascii_uppercase())),
        _ => None,
    }
}

/// The drives that must never be erased: the one Windows runs from, the one this app runs from and the one
/// that holds its checklist.
pub fn protected_letters() -> Vec<String> {
    let mut letters = vec![system::system_drive().to_ascii_uppercase()];
    letters.extend(drive_of(&system::exe_path()));
    letters.extend(drive_of(&crate::state::data_file()));
    letters.sort();
    letters.dedup();
    letters
}

pub fn is_candidate(disk: &Disk, protected: &[String]) -> bool {
    matches!(disk.bus, BUS_USB | BUS_SD | BUS_MMC)
        && disk.size > 0
        && !disk.letters().iter().any(|letter| protected.iter().any(|other| other.eq_ignore_ascii_case(letter)))
}

/// The USB drives and memory cards that can be reset.
pub fn candidates() -> Vec<Disk> {
    let protected = protected_letters();
    hardware::disks().into_iter().filter(|disk| is_candidate(disk, &protected)).collect()
}

/// How much is stored on the drive, as far as Windows can read it.
pub fn used(disk: &Disk) -> u64 {
    disk.volumes.iter().filter_map(|volume| volume.space).map(|(free, total)| total.saturating_sub(free)).sum()
}

/// Whether to warn that this could be a backup drive: an external disk rather than a stick, or a big one.
pub fn looks_like_backup(disk: &Disk) -> bool {
    (disk.bus == BUS_USB && !disk.removable_media) || disk.size > LARGE
}

/// A name for the drive in the window, such as "SanDisk Ultra · 57.3 GB · E:".
pub fn describe(disk: &Disk) -> String {
    let name = if disk.model.is_empty() { format!("Disk {}", disk.number) } else { disk.model.clone() };
    let mut parts = vec![name, system::size(disk.size)];
    let letters = disk.letters();
    if !letters.is_empty() {
        parts.push(letters.join(" "));
    }
    parts.join(" · ")
}

/// Text in PowerShell's single quotes, where only a single quote needs escaping, by doubling it.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

// What the script's exit codes mean.
const EXIT_DONE: u32 = 0;
const EXIT_FAILED: u32 = 1;
const EXIT_CHANGED: u32 = 3;
const EXIT_READ_ONLY: u32 = 4;

/// The PowerShell script that erases the drive and sets it up again. Before it touches anything, it checks
/// again that the disk is still the USB drive or memory card that was picked, by its number, bus and exact
/// size, and that neither Windows nor a protected drive letter is on it. Any error message goes to
/// `message_file`, since a program run as administrator has no output this app can read.
pub fn script(disk: &Disk, file_system: FileSystem, label: &str, protected: &[String], message_file: &Path) -> String {
    let protected: String = protected
        .iter()
        .filter_map(|letter| letter.chars().next().filter(char::is_ascii_alphabetic))
        .map(|letter| format!(" -or (Get-Partition -DriveLetter {letter} -ErrorAction SilentlyContinue).DiskNumber -eq $n"))
        .collect();
    let (style, mbr_type) = if disk.size > MBR_LIMIT {
        ("GPT", String::new())
    } else {
        ("MBR", format!(" -MbrType {}", if file_system == FileSystem::Fat32 { "FAT32" } else { "IFS" }))
    };
    let label = quote(&clean_label(label, file_system));
    let message_file = quote(&message_file.to_string_lossy());
    format!(
        "$ErrorActionPreference='Stop';$n={number};try{{$d=Get-Disk -Number $n;\
         if($d.BusType -notin 'USB','SD','MMC' -or $d.Size -ne {size} -or $d.IsBoot -or $d.IsSystem{protected}){{exit {EXIT_CHANGED}}}\
         if($d.IsReadOnly){{try{{Set-Disk -Number $n -IsReadOnly $false}}catch{{}};if((Get-Disk -Number $n).IsReadOnly){{exit {EXIT_READ_ONLY}}}}}\
         if($d.IsOffline){{Set-Disk -Number $n -IsOffline $false}}\
         if($d.PartitionStyle -ne 'RAW'){{Clear-Disk -Number $n -RemoveData -RemoveOEM -Confirm:$false}}\
         Initialize-Disk -Number $n -PartitionStyle {style};\
         $p=New-Partition -DiskNumber $n -UseMaximumSize{mbr_type};\
         $p|Format-Volume -FileSystem {file_system} -NewFileSystemLabel {label} -Force -Confirm:$false|Out-Null;\
         try{{$p|Add-PartitionAccessPath -AssignDriveLetter}}catch{{}};exit {EXIT_DONE}}}\
         catch{{Set-Content -LiteralPath {message_file} -Value $_.Exception.Message -Encoding UTF8;exit {EXIT_FAILED}}}",
        number = disk.number,
        size = disk.size,
        file_system = file_system.name(),
    )
}

/// A file name nobody can guess, for the script's error message.
fn message_file() -> PathBuf {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos()));
    hasher.write_u32(std::process::id());
    std::env::temp_dir().join(format!("UpdateCheck-usb-{:016x}.txt", hasher.finish()))
}

const NOTHING_CHANGED: &str = "Nothing was erased.";

/// What went wrong, for the window.
fn failure(code: u32, message: &str) -> String {
    match code {
        EXIT_CHANGED => format!("The drive isn't the one you picked anymore. {NOTHING_CHANGED}"),
        EXIT_READ_ONLY => format!(
            "The drive is write-protected. If it has a small lock switch, slide it to unlock, then try again. \
             {NOTHING_CHANGED}"
        ),
        _ if message.trim().is_empty() => format!("Windows couldn't reset the drive (code {code})."),
        _ => format!("Windows couldn't reset the drive: {}", message.trim()),
    }
}

/// Erase the drive and set it up again, as administrator. The drive is looked up again first, so a stick that
/// was swapped for another one since it was picked is never erased. Returns the new drive letter, if Windows
/// has given it one yet, or that the user said no to administrator rights; `Err` is for failures.
pub fn reset(picked: &Disk, file_system: FileSystem, label: &str) -> Result<ResetOutcome, String> {
    let protected = protected_letters();
    let still_there = hardware::disks().into_iter().find(|disk| {
        disk.number == picked.number && disk.size == picked.size && disk.model == picked.model && disk.bus == picked.bus
    });
    let Some(disk) = still_there.filter(|disk| is_candidate(disk, &protected)) else {
        return Err(format!("The drive isn't there anymore, or it changed. {NOTHING_CHANGED}"));
    };
    if !file_system.fits(disk.size) {
        return Err(format!("{} works only on drives up to 32 GB. {NOTHING_CHANGED}", file_system.name()));
    }
    let message_file = message_file();
    let _ = std::fs::remove_file(&message_file);
    let script = script(&disk, file_system, label, &protected, &message_file);
    let parameters = format!("-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command {script}");
    applog::info("usb", format!("Resetting disk {} ({}) as {}", disk.number, describe(&disk), file_system.name()));
    let result = system::run_elevated(&system::powershell(), &parameters, Duration::from_secs(600));
    let message = std::fs::read_to_string(&message_file).unwrap_or_default();
    let _ = std::fs::remove_file(&message_file);
    let message = message.trim_start_matches('\u{feff}');
    match result {
        Err(system::ERROR_CANCELLED) => Ok(ResetOutcome::Cancelled),
        Err(code) => Err(format!("Windows couldn't start the reset: {}", system::error_text(code))),
        Ok(None) => Err("Resetting is taking unusually long. Leave the drive plugged in until its light stops \
                         blinking, then look for it in File Explorer."
            .to_owned()),
        Ok(Some(EXIT_DONE)) => {
            applog::info("usb", format!("Disk {} was reset", disk.number));
            Ok(ResetOutcome::Done(new_letter(disk.number)))
        }
        Ok(Some(code)) => {
            applog::warning("usb", format!("Resetting disk {} failed with {code}: {}", disk.number, message.trim()));
            Err(failure(code, message))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetOutcome {
    /// Reset, with the drive letter Windows gave it, if it has given one yet.
    Done(Option<String>),
    /// The user said no to administrator rights.
    Cancelled,
}

/// The letter Windows gives the drive once it is set up again, which can take it a moment.
fn new_letter(number: u32) -> Option<String> {
    for _ in 0..10 {
        let letter = hardware::disks().into_iter().find(|disk| disk.number == number).and_then(|disk| disk.letters().into_iter().next());
        if letter.is_some() {
            return letter;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BUS_NVME, Volume};

    fn stick(letter: &str) -> Disk {
        Disk {
            number: 2,
            model: "SanDisk Ultra".to_owned(),
            serial: String::new(),
            size: 61_530_439_680,
            bus: BUS_USB,
            removable_media: true,
            solid_state: None,
            volumes: vec![Volume {
                paths: vec![format!("{letter}\\")],
                label: "My stick!".to_owned(),
                file_system: "FAT32".to_owned(),
                space: Some((40_000_000_000, 61_000_000_000)),
            }],
        }
    }

    #[test]
    fn only_usb_drives_and_cards_windows_does_not_run_from_are_offered() {
        let protected = ["C:".to_owned(), "D:".to_owned()];
        assert!(is_candidate(&stick("E:"), &protected));
        assert!(is_candidate(&Disk { bus: BUS_SD, ..stick("F:") }, &protected));
        assert!(!is_candidate(&stick("C:"), &protected));
        assert!(!is_candidate(&stick("d:"), &protected));
        assert!(!is_candidate(&Disk { bus: BUS_NVME, ..stick("E:") }, &protected));
        // A card reader without a card in it.
        assert!(!is_candidate(&Disk { size: 0, volumes: Vec::new(), ..stick("E:") }, &protected));
    }

    #[test]
    fn labels_are_what_the_file_system_takes() {
        assert_eq!(clean_label("My stick!", FileSystem::ExFat), "My stick");
        assert_eq!(clean_label("  Fotos   2026 ", FileSystem::Fat32), "FOTOS 2026");
        assert_eq!(clean_label("Backup drive for the office", FileSystem::ExFat), "Backup driv");
        assert_eq!(clean_label("Backup drive for the office", FileSystem::Ntfs), "Backup drive for the office");
        assert_eq!(clean_label("Grüße 'x'", FileSystem::Ntfs), "Gre x");
        assert_eq!(clean_label("a b c d e f g", FileSystem::ExFat), "a b c d e f");
        assert_eq!(default_label(&stick("E:"), FileSystem::Fat32), "MY STICK");
        assert_eq!(default_label(&Disk { volumes: Vec::new(), ..stick("E:") }, FileSystem::ExFat), "USB");
    }

    #[test]
    fn fat32_only_fits_small_drives() {
        assert!(FileSystem::Fat32.fits(31_000_000_000));
        assert!(!FileSystem::Fat32.fits(61_530_439_680));
        assert!(FileSystem::ExFat.fits(61_530_439_680) && FileSystem::Ntfs.fits(4_000_000_000_000));
    }

    #[test]
    fn the_script_checks_the_drive_again_before_erasing() {
        let protected = ["C:".to_owned(), "D:".to_owned()];
        let text = script(&stick("E:"), FileSystem::ExFat, "My 'stick'", &protected, Path::new(r"C:\Users\O'Brien\AppData\Local\Temp\m.txt"));
        let check = text.find("$d.Size -ne 61530439680").unwrap();
        assert!(check < text.find("Clear-Disk").unwrap());
        assert!(text.contains("$n=2;"));
        assert!(text.contains("(Get-Partition -DriveLetter C -ErrorAction SilentlyContinue).DiskNumber -eq $n"));
        assert!(text.contains("(Get-Partition -DriveLetter D -ErrorAction SilentlyContinue).DiskNumber -eq $n"));
        assert!(text.contains("-PartitionStyle MBR;") && text.contains("-MbrType IFS;"));
        assert!(text.contains("-FileSystem exFAT -NewFileSystemLabel 'My stick' "));
        assert!(text.contains(r"-LiteralPath 'C:\Users\O''Brien\AppData\Local\Temp\m.txt'"));
        // PowerShell gets the script as plain words on its command line, where double quotes would be eaten.
        assert!(!text.contains('"'));
        assert!(text.len() < 1500, "{} characters is too long for ShellExecute", text.len());
        let big = Disk { size: 4_000_000_000_000, removable_media: false, ..stick("E:") };
        let text = script(&big, FileSystem::Ntfs, "", &protected, Path::new("m.txt"));
        assert!(text.contains("-PartitionStyle GPT;") && !text.contains("-MbrType"));
        let text = script(&Disk { size: 30_000_000_000, ..stick("E:") }, FileSystem::Fat32, "x", &protected, Path::new("m.txt"));
        assert!(text.contains("-MbrType FAT32;") && text.contains("-NewFileSystemLabel 'X'"));
    }

    #[test]
    fn external_disks_and_big_drives_get_a_backup_warning() {
        assert!(!looks_like_backup(&stick("E:")));
        assert!(looks_like_backup(&Disk { removable_media: false, ..stick("E:") }));
        assert!(looks_like_backup(&Disk { size: 1_000_000_000_000, ..stick("E:") }));
        assert!(!looks_like_backup(&Disk { bus: BUS_SD, removable_media: false, ..stick("E:") }));
    }

    #[test]
    fn drives_are_described_by_name_size_and_letter() {
        assert_eq!(describe(&stick("E:")), format!("SanDisk Ultra · {} · E:", system::size(61_530_439_680)));
        assert_eq!(used(&stick("E:")), 21_000_000_000);
        assert_eq!(drive_of(Path::new(r"\\?\d:\Apps\UpdateCheck.exe")).as_deref(), Some("D:"));
        assert_eq!(drive_of(Path::new(r"\\server\share\x.exe")), None);
    }

    #[test]
    fn failures_say_that_nothing_was_erased() {
        assert!(failure(EXIT_CHANGED, "").ends_with(NOTHING_CHANGED));
        assert!(failure(EXIT_READ_ONLY, "").contains("lock switch"));
        assert_eq!(failure(EXIT_FAILED, "Zugriff verweigert\r\n"), "Windows couldn't reset the drive: Zugriff verweigert");
        assert_eq!(failure(EXIT_FAILED, ""), "Windows couldn't reset the drive (code 1).");
    }
}

#[cfg(test)]
mod dry_run {
    use std::os::windows::process::CommandExt;

    /// Runs the script the way the reset does, but without administrator rights and for a disk that doesn't
    /// exist, with a size no disk has, so it stops at its first check: `cargo test dry_run -- --ignored`.
    #[test]
    #[ignore]
    fn the_script_runs_and_reports_errors() {
        let disk = crate::hardware::Disk {
            number: 63,
            model: "None".to_owned(),
            serial: String::new(),
            size: 1,
            bus: crate::hardware::BUS_USB,
            removable_media: true,
            solid_state: None,
            volumes: Vec::new(),
        };
        let message_file = super::message_file();
        let protected = ["C:".to_owned()];
        let script = super::script(&disk, super::FileSystem::ExFat, "x", &protected, &message_file);
        let status = std::process::Command::new(crate::system::powershell())
            .raw_arg(format!("-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command {script}"))
            .creation_flags(crate::system::CREATE_NO_WINDOW)
            .status()
            .unwrap();
        let message = std::fs::read_to_string(&message_file).unwrap_or_default();
        let _ = std::fs::remove_file(&message_file);
        println!("exit {:?}, message {message:?}", status.code());
        assert_eq!(status.code(), Some(super::EXIT_FAILED as i32));
        assert!(!message.trim_start_matches('\u{feff}').trim().is_empty());
    }
}
