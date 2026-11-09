//! What this PC is, for the PC info tab: maker and model, processor, memory, graphics, drives, Windows, who
//! manages the PC, and its firmware.
//!
//! Everything comes from the registry, the firmware's tables and a few Windows API calls, so reading it all
//! takes milliseconds, needs no administrator rights and flashes up no console. Whatever can't be read is left
//! out rather than guessed.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut};

use chrono::{Datelike, NaiveDate};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::core::BOOL;
use windows_sys::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, NetGetJoinInformation, NetSetupDomainName};
use windows_sys::Win32::System::SystemInformation::{GetFirmwareType, GetPhysicallyInstalledSystemMemory};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::TpmBaseServices::Tbsi_GetDeviceInfo;

use crate::graphics::{self, Adapter};
use crate::hardware::{self, Disk, Memory, MemoryModule, Topology};
use crate::system::{self, Hive, size};

/// When a row is shown: in either view, only in the short one, or only with details turned on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Always,
    Summary,
    Details,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    /// Maybe running over several lines.
    pub value: String,
    pub show: Show,
}

impl Row {
    fn new(label: impl Into<String>, value: impl Into<String>, show: Show) -> Row {
        Row { label: label.into(), value: value.into(), show }
    }

    fn always(label: impl Into<String>, value: impl Into<String>) -> Row {
        Row::new(label, value, Show::Always)
    }

    fn details(label: impl Into<String>, value: impl Into<String>) -> Row {
        Row::new(label, value, Show::Details)
    }

    pub fn visible(&self, details: bool) -> bool {
        match self.show {
            Show::Always => true,
            Show::Summary => !details,
            Show::Details => details,
        }
    }
}

/// A card of the tab: its title, its rows, and perhaps a button under them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
    /// The button's label, and the page or settings it opens.
    pub action: Option<(&'static str, String)>,
}

impl Section {
    fn new(title: &'static str, rows: Vec<Row>) -> Section {
        Section { title, rows, action: None }
    }

    pub fn visible_rows(&self, details: bool) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(move |row| row.visible(details))
    }

    pub fn is_visible(&self, details: bool) -> bool {
        self.visible_rows(details).next().is_some()
    }
}

const BIOS: &str = r"HARDWARE\DESCRIPTION\System\BIOS";
const BIOS_VALUES: [&str; 7] = [
    "SystemManufacturer",
    "SystemProductName",
    "SystemVersion",
    "BaseBoardManufacturer",
    "BaseBoardProduct",
    "BIOSVersion",
    "BIOSReleaseDate",
];
const PROCESSOR: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
const WINDOWS: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const SECURE_BOOT: &str = r"SYSTEM\CurrentControlSet\Control\SecureBoot\State";
/// Where Windows keeps, for each version it was upgraded from, a key named "Source OS (Updated on ...)".
const SETUP: &str = r"SYSTEM\Setup";
/// Either one is there while Windows waits for a restart to finish installing updates.
const RESTART_KEYS: [&str; 2] = [
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
];
/// What PCs built from parts report instead of a maker or a model.
const PLACEHOLDERS: [&str; 8] = [
    "",
    "default string",
    "none",
    "not applicable",
    "o.e.m.",
    "system manufacturer",
    "system product name",
    "to be filled by o.e.m.",
];
/// Makers whose support sites find the downloads for the PC they are opened on, by the start of their name.
const SUPPORT_SITES: [(&str, &str); 5] = [
    ("DELL", "https://www.dell.com/support/home/"),
    ("HEWLETT-PACKARD", "https://support.hp.com/drivers"),
    ("HP", "https://support.hp.com/drivers"),
    ("LENOVO", "https://pcsupport.lenovo.com/"),
    // Surface firmware comes through Windows Update, as does a Hyper-V virtual machine's.
    ("MICROSOFT", "ms-settings:windowsupdate"),
];
const TPM_NOT_FOUND: u32 = 0x8028_400F;

/// Text values of a key by name; a value that can't be read is left out.
pub type Values = HashMap<&'static str, String>;

/// Every section of the tab, read now. `updates` is what Windows Update said, which takes too long to ask here.
pub fn gather(updates: &str, today: NaiveDate) -> Vec<Section> {
    let bios = values(BIOS, &BIOS_VALUES);
    let windows = values(WINDOWS, &["ProductName", "CurrentBuild", "DisplayVersion", "ReleaseId"]);
    let build = text(&windows, "CurrentBuild");
    let ubr = system::reg_dword(Hive::LocalMachine, WINDOWS, "UBR");
    let install_date = system::reg_dword(Hive::LocalMachine, WINDOWS, "InstallDate");

    let processor = system::reg_string(Hive::LocalMachine, PROCESSOR, "ProcessorNameString")
        .map(|name| collapse(&name))
        .unwrap_or_default();
    let topology = hardware::topology();
    let threads = hardware::threads();
    let installed = installed_memory();
    let adapters = graphics::adapters().unwrap_or_default();

    let processor_details =
        processor_rows(&processor, &topology, threads, system::reg_dword(Hive::LocalMachine, PROCESSOR, "~MHz"));
    let memory_details = memory_rows(installed, &hardware::memory());
    let graphics_details = graphics_rows(&adapters);
    let drive_details = drive_rows(&hardware::disks());
    // A row of the short view gives way to its section of details, unless there is nothing in that.
    let summary = |details: &[Row]| if details.is_empty() { Show::Always } else { Show::Summary };

    let sections = vec![
        Section::new(
            "Device",
            vec![
                Row::always("Name", std::env::var("COMPUTERNAME").unwrap_or_default()),
                Row::always("Model", model(&bios)),
                Row::new("Processor", processor_summary(&processor, topology.core_count(), threads), summary(&processor_details)),
                Row::new("Memory", installed.map(size).unwrap_or_default(), summary(&memory_details)),
                Row::new(
                    "Graphics",
                    adapters.iter().map(Adapter::label).collect::<Vec<_>>().join("\n"),
                    summary(&graphics_details),
                ),
                Row::new("System drive", system_drive(), summary(&drive_details)),
            ],
        ),
        Section::new("Processor", processor_details),
        Section::new("Memory", memory_details),
        Section::new("Graphics", graphics_details),
        Section::new("Drives", drive_details),
        Section::new(
            "Windows",
            vec![
                Row::always("Edition", windows_name(&text(&windows, "ProductName"), build.parse().unwrap_or(0))),
                Row::always("Version", Some(text(&windows, "DisplayVersion")).filter(|v| !v.is_empty()).unwrap_or_else(|| text(&windows, "ReleaseId"))),
                Row::always("OS build", match ubr {
                    Some(ubr) if !build.is_empty() => format!("{build}.{ubr}"),
                    _ => build.clone(),
                }),
                Row::always("Updates", updates),
                Row::always("Installed on", installed_on(first_installed(install_date, &earlier_installs()))),
                Row::always("System type", system_type()),
                Row::always("Managed by", managed_by()),
            ],
        ),
        Section {
            title: "Firmware",
            rows: vec![
                Row::always("BIOS", bios_version(&bios, today)),
                Row::details("Motherboard", motherboard(&bios)),
                Row::always("Boot mode", boot_mode()),
                Row::always("Secure Boot", secure_boot()),
                Row::always("TPM", tpm()),
            ],
            action: Some(("Find BIOS updates", bios_page(&bios))),
        },
    ];
    // Whatever couldn't be read is left out, and so is a section with nothing left in it.
    sections
        .into_iter()
        .map(|mut section| {
            section.rows.retain(|row| !row.value.is_empty());
            section
        })
        .filter(|section| !section.rows.is_empty())
        .collect()
}

/// The sections as plain text for the clipboard, as far as they are shown.
pub fn as_text(sections: &[Section], details: bool) -> String {
    sections
        .iter()
        .filter(|section| section.is_visible(details))
        .map(|section| {
            std::iter::once(section.title.to_owned())
                .chain(section.visible_rows(details).map(|row| format!("{}: {}", row.label, row.value.replace('\n', ", "))))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Whether Windows waits for a restart to finish installing updates.
pub fn restart_pending() -> bool {
    RESTART_KEYS.iter().any(|key| system::reg_key_exists(Hive::LocalMachine, key))
}

// --- Formatting -------------------------------------------------------------------------------------------

/// The PC's maker and model. A PC built from parts names its mainboard instead, and Lenovo keeps the model in
/// the version, its product name being an order number.
pub fn model(bios: &Values) -> String {
    let (mut maker, mut product) = (named(bios, "SystemManufacturer"), named(bios, "SystemProductName"));
    if maker.eq_ignore_ascii_case("LENOVO") {
        let version = named(bios, "SystemVersion");
        if !version.is_empty() {
            product = version;
        }
    }
    if product.is_empty() {
        (maker, product) = (named(bios, "BaseBoardManufacturer"), named(bios, "BaseBoardProduct"));
    }
    join_maker(&maker, &product)
}

/// The mainboard, where it isn't what the model names already.
fn motherboard(bios: &Values) -> String {
    let board = join_maker(&named(bios, "BaseBoardManufacturer"), &named(bios, "BaseBoardProduct"));
    if board == model(bios) { String::new() } else { board }
}

fn join_maker(maker: &str, product: &str) -> String {
    // HP's product names already start with "HP".
    if product.to_lowercase().starts_with(&maker.to_lowercase()) {
        return product.to_owned();
    }
    format!("{maker} {product}").trim().to_owned()
}

/// Windows 11 still calls itself Windows 10 in the registry; only its build number, 22000 or later, tells.
pub fn windows_name(product: &str, build: u32) -> String {
    match product.strip_prefix("Windows 10") {
        Some(rest) if build >= 22000 => format!("Windows 11{rest}"),
        _ => product.to_owned(),
    }
}

/// The BIOS version, the day it was released and how long ago that was. The BIOS gives the day as
/// month/day/year.
pub fn bios_version(bios: &Values, today: NaiveDate) -> String {
    let version = text(bios, "BIOSVersion");
    let mut released = text(bios, "BIOSReleaseDate");
    let parts: Vec<Option<u32>> = released.split('/').map(|part| part.trim().parse().ok()).collect();
    if let [Some(month), Some(day), Some(year)] = parts[..]
        && let Some(day) = NaiveDate::from_ymd_opt(year as i32, month, day) {
            released = format!("{} ({})", date_text(day), age(day, today));
        }
    [version, released].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// How old something from `day` is, in months, or in years from two years on.
pub fn age(day: NaiveDate, today: NaiveDate) -> String {
    let months = (today.year() - day.year()) * 12 + today.month() as i32 - day.month() as i32
        - i32::from(today.day() < day.day());
    if months < 1 {
        "less than a month old".to_owned()
    } else if months < 24 {
        format!("{months} month{} old", if months > 1 { "s" } else { "" })
    } else {
        format!("{} years old", months / 12)
    }
}

/// Where to look for a newer BIOS. Neither Windows nor the BIOS can tell whether there is one, and every maker
/// publishes them differently: so this is a support site that finds the PC's downloads by itself, or else a
/// web search for the model's BIOS.
pub fn bios_page(bios: &Values) -> String {
    let maker = named(bios, "SystemManufacturer").to_uppercase();
    if let Some((_, site)) = SUPPORT_SITES.iter().find(|(name, _)| maker.starts_with(name)) {
        return (*site).to_owned();
    }
    format!("https://duckduckgo.com/?q={}", quote_plus(format!("{} BIOS update", model(bios)).trim()))
}

/// Text for a web address's query, as a form would send it.
pub fn quote_plus(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => quoted.push(char::from(byte)),
            b' ' => quoted.push('+'),
            _ => quoted.push_str(&format!("%{byte:02X}")),
        }
    }
    quoted
}

/// Who manages the PC: an Active Directory domain, a Microsoft Entra ID tenant ("" for one without a name),
/// both, or nobody.
pub fn management(domain: Option<&str>, tenant: Option<&str>) -> String {
    let mut parts = Vec::new();
    match tenant {
        Some("") => parts.push("Microsoft Entra ID".to_owned()),
        Some(tenant) => parts.push(format!("{tenant} (Microsoft Entra ID)")),
        None => {}
    }
    if let Some(domain) = domain.filter(|domain| !domain.is_empty()) {
        parts.push(format!("{domain} (Active Directory)"));
    }
    if parts.is_empty() { "Not managed by an organization".to_owned() } else { parts.join("\n") }
}

/// When Windows was first installed on this PC, in seconds since 1970. Windows moves its install date to the
/// day of each upgrade to a new version, so the earliest of the current and the earlier versions' dates wins.
pub fn first_installed(current: Option<u32>, earlier: &[Option<u32>]) -> Option<u32> {
    std::iter::once(current).chain(earlier.iter().copied()).flatten().filter(|stamp| *stamp > 0).min()
}

/// A day as the tab shows it, such as "Sep 25, 2025".
pub fn date_text(day: NaiveDate) -> String {
    format!("{} {}, {}", day.format("%b"), day.day(), day.year())
}

fn installed_on(timestamp: Option<u32>) -> String {
    timestamp
        .and_then(|stamp| chrono::DateTime::from_timestamp(i64::from(stamp), 0))
        .map(|time| date_text(time.with_timezone(&chrono::Local).date_naive()))
        .unwrap_or_default()
}

pub fn processor_summary(name: &str, cores: usize, threads: usize) -> String {
    if cores == 0 || threads == 0 {
        return name.to_owned();
    }
    let count = if threads != cores { format!("{cores} cores, {threads} threads") } else { format!("{cores} cores") };
    if name.is_empty() { count } else { format!("{name}\n{count}") }
}

pub fn processor_rows(name: &str, topology: &Topology, threads: usize, megahertz: Option<u32>) -> Vec<Row> {
    let mut rows = vec![
        Row::details("Name", name),
        Row::details("Cores", core_text(&topology.cores)),
        Row::details("Threads", if threads > 0 { threads.to_string() } else { String::new() }),
        Row::details(
            "Base speed",
            megahertz.filter(|speed| *speed > 0).map_or_else(String::new, |speed| format!("{:.2} GHz", f64::from(speed) / 1000.0)),
        ),
        Row::details("Cache", cache_text(&topology.caches)),
    ];
    if topology.packages > 1 {
        rows.push(Row::details("Processors", format!("{} sockets", topology.packages)));
    }
    rows.retain(|row| !row.value.is_empty());
    rows
}

/// The cores, by kind where the processor mixes them: "12 (8 performance + 4 efficient)".
pub fn core_text(cores: &[usize]) -> String {
    const KINDS: [&str; 3] = ["performance", "efficient", "low-power efficient"];
    let total: usize = cores.iter().sum();
    match cores {
        [] => String::new(),
        [_] => total.to_string(),
        _ => {
            let kinds: Vec<String> = cores
                .iter()
                .enumerate()
                .map(|(index, count)| format!("{count} {}", KINDS.get(index).copied().unwrap_or("other")))
                .collect();
            format!("{total} ({})", kinds.join(" + "))
        }
    }
}

pub fn cache_text(caches: &[u64; 3]) -> String {
    caches
        .iter()
        .enumerate()
        .filter(|(_, bytes)| **bytes > 0)
        .map(|(level, bytes)| format!("L{} {}", level + 1, cache_size(*bytes)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn cache_size(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if bytes < MB {
        return format!("{} KB", bytes.div_ceil(1024));
    }
    let text = format!("{:.2}", bytes as f64 / MB as f64);
    format!("{} MB", text.trim_end_matches('0').trim_end_matches('.'))
}

/// The memory modules: who made each and which one it is, its size, kind and speed, and the slot it is in.
pub fn memory_rows(installed: Option<u64>, memory: &Memory) -> Vec<Row> {
    if memory.modules.is_empty() {
        return Vec::new();
    }
    let total = installed.unwrap_or_else(|| memory.modules.iter().map(|module| module.size).sum());
    let mut rows = vec![Row::details("Installed", size(total))];
    let (soldered, sticks): (Vec<&MemoryModule>, Vec<&MemoryModule>) =
        memory.modules.iter().partition(|module| module.soldered);
    // Firmware counts soldered memory in slots of its own, which are none a module could go in.
    if soldered.is_empty() && memory.slots >= sticks.len() {
        rows.push(Row::details("Slots", format!("{} of {} used", sticks.len(), memory.slots)));
    }
    for (index, module) in sticks.iter().enumerate() {
        let identity = [module.maker.as_deref(), module.part.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let mut details = module_spec(module.size, module.kind, module.speed);
        if !module.slot.is_empty() {
            details.push_str(&format!(" · slot {}", module.slot));
        }
        let value = if identity.is_empty() { details } else { format!("{identity}\n{details}") };
        rows.push(Row::details(format!("Module {}", index + 1), value));
    }
    if let Some(first) = soldered.first() {
        let mut makers: Vec<&str> = soldered.iter().filter_map(|module| module.maker.as_deref()).collect();
        makers.sort_unstable();
        makers.dedup();
        let total = soldered.iter().map(|module| module.size).sum();
        let details = format!("{}, soldered to the board", module_spec(total, first.kind, first.speed));
        rows.push(Row::details(
            "Built in",
            if makers.is_empty() { details } else { format!("{}\n{details}", makers.join(", ")) },
        ));
    }
    rows
}

/// "16 GB DDR5-6000", as memory is sold.
fn module_spec(bytes: u64, kind: Option<&str>, speed: Option<u32>) -> String {
    match (kind, speed) {
        (Some(kind), Some(speed)) => format!("{} {kind}-{speed}", size(bytes)),
        (Some(kind), None) => format!("{} {kind}", size(bytes)),
        (None, Some(speed)) => format!("{} at {speed} MT/s", size(bytes)),
        (None, None) => size(bytes),
    }
}

/// Each graphics adapter with its own memory and its driver.
pub fn graphics_rows(adapters: &[Adapter]) -> Vec<Row> {
    adapters
        .iter()
        .enumerate()
        .map(|(index, adapter)| {
            let mut lines = vec![adapter.label()];
            if let Some(memory) = adapter.memory {
                lines.push(format!("{} video memory", video_memory(memory)));
            }
            if !adapter.driver_version.is_empty() {
                let version = adapter.card().map_or_else(|| adapter.driver_version.clone(), |card| card.version());
                lines.push(match adapter.driver_date {
                    Some(day) => format!("Driver {version} from {}", date_text(day)),
                    None => format!("Driver {version}"),
                });
            }
            let label = if adapters.len() > 1 { format!("GPU {}", index + 1) } else { "GPU".to_owned() };
            Row::details(label, lines.join("\n"))
        })
        .collect()
}

/// Video memory as cards are sold: the driver reports a little less than a card has, which keeps some for
/// itself, so 15.9 GB is a 16 GB card. Sizes come in steps of half a gigabyte at the least.
fn video_memory(bytes: u64) -> String {
    const HALF_GB: u64 = 512 * 1024 * 1024;
    if bytes < 2 * HALF_GB {
        return size(bytes);
    }
    let halves = (bytes + HALF_GB / 2) / HALF_GB;
    if halves.is_multiple_of(2) { format!("{} GB", halves / 2) } else { format!("{}.5 GB", halves / 2) }
}

/// Each disk with its size and kind, and the space on each of its drive letters.
pub fn drive_rows(disks: &[Disk]) -> Vec<Row> {
    disks
        .iter()
        .map(|disk| {
            let mut lines = Vec::new();
            if !disk.model.is_empty() {
                lines.push(disk.model.clone());
            }
            lines.push(format!("{} {}", size(disk.size), disk.kind()));
            let mut volumes: Vec<(&str, &hardware::Volume)> = disk
                .volumes
                .iter()
                .flat_map(|volume| volume.paths.iter().map(move |path| (path.as_str(), volume)))
                .filter(|(path, _)| path.len() == 3 && path.ends_with(":\\"))
                .collect();
            volumes.sort_by_key(|(path, _)| *path);
            for (path, volume) in &volumes {
                let mut line = path[..2].to_owned();
                if !volume.label.is_empty() {
                    line.push_str(&format!(" {},", volume.label));
                }
                if let Some((free, total)) = volume.space {
                    line.push_str(&format!(" {} free of {}", size(free), size(total)));
                }
                lines.push(line.trim_end_matches(',').to_owned());
            }
            if volumes.is_empty() {
                lines.push("No drive letter".to_owned());
            }
            Row::details(format!("Disk {}", disk.number), lines.join("\n"))
        })
        .collect()
}

// --- Reading ----------------------------------------------------------------------------------------------

fn values(key: &str, names: &[&'static str]) -> Values {
    names
        .iter()
        .filter_map(|name| system::reg_string(Hive::LocalMachine, key, name).map(|value| (*name, value)))
        .collect()
}

/// Text with its runs of spaces made single, as processor names come padded with them.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text(values: &Values, name: &str) -> String {
    values.get(name).map(|value| collapse(value)).unwrap_or_default()
}

fn named(values: &Values, name: &str) -> String {
    let text = text(values, name);
    if PLACEHOLDERS.contains(&text.to_lowercase().as_str()) { String::new() } else { text }
}

fn installed_memory() -> Option<u64> {
    let mut kilobytes = 0u64;
    (unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) } != 0 && kilobytes > 0).then_some(kilobytes * 1024)
}

fn system_drive() -> String {
    system::system_drive_space()
        .map(|(drive, free, total)| format!("{} free of {} ({drive})", size(free), size(total)))
        .unwrap_or_default()
}

/// The install dates of the versions of Windows this PC was upgraded from.
fn earlier_installs() -> Vec<Option<u32>> {
    system::reg_subkeys(Hive::LocalMachine, SETUP)
        .iter()
        .filter(|name| name.starts_with("Source OS"))
        .map(|name| system::reg_dword(Hive::LocalMachine, &format!(r"{SETUP}\{name}"), "InstallDate"))
        .collect()
}

/// 32- or 64-bit Windows and the processor's architecture, which tells an ARM PC from the rest. Asked of
/// Windows itself, since an x64 program running on an ARM PC is told it's on x64.
fn system_type() -> String {
    type IsWow64Process2 = unsafe extern "system" fn(HANDLE, *mut u16, *mut u16) -> BOOL;
    let Some(check) = (unsafe { system::system_function::<IsWow64Process2>("kernel32.dll", c"IsWow64Process2") }) else {
        return String::new();
    };
    let (mut process, mut native) = (0u16, 0u16);
    if unsafe { check(GetCurrentProcess(), &mut process, &mut native) } == 0 {
        return String::new();
    }
    let machine = match native {
        0x014C => "x86",
        0x8664 => "x64",
        0xAA64 => "ARM64",
        _ => return String::new(),
    };
    format!("{}-bit Windows, {machine} processor", if machine == "x86" { "32" } else { "64" })
}

fn managed_by() -> String {
    management(domain().as_deref(), entra_tenant().as_deref())
}

/// The Active Directory domain the PC is joined to, if any. A workgroup isn't one.
fn domain() -> Option<String> {
    let mut name: *mut u16 = null_mut();
    let mut status = 0;
    if unsafe { NetGetJoinInformation(null(), &mut name, &mut status) } != 0 {
        return None;
    }
    let domain = (status == NetSetupDomainName).then(|| unsafe { system::from_wide_pointer(name) });
    if !name.is_null() {
        unsafe { NetApiBufferFree(name.cast()) };
    }
    domain
}

/// The start of DSREG_JOIN_INFO, as far as the tenant's name.
#[repr(C)]
struct JoinInfo {
    join_type: i32,
    certificate: *mut c_void,
    device_id: *mut u16,
    idp_domain: *mut u16,
    tenant_id: *mut u16,
    user_email: *mut u16,
    tenant_name: *mut u16,
}

/// The name of the Microsoft Entra ID tenant the PC is joined to: "" for a tenant without one, None if it isn't
/// joined. A work account merely added to a personal PC doesn't put the organization in charge of it.
fn entra_tenant() -> Option<String> {
    type Get = unsafe extern "system" fn(*const u16, *mut *mut JoinInfo) -> i32;
    type Free = unsafe extern "system" fn(*const JoinInfo);
    const DEVICE_JOIN: i32 = 1;
    let get = unsafe { system::system_function::<Get>("netapi32.dll", c"NetGetAadJoinInformation") }?;
    let free = unsafe { system::system_function::<Free>("netapi32.dll", c"NetFreeAadJoinInformation") }?;
    let mut info: *mut JoinInfo = null_mut();
    if unsafe { get(null(), &mut info) } != 0 || info.is_null() {
        return None;
    }
    let tenant = unsafe { ((*info).join_type == DEVICE_JOIN).then(|| system::from_wide_pointer((*info).tenant_name)) };
    unsafe { free(info) };
    tenant
}

fn boot_mode() -> String {
    let mut kind = 0;
    let read = unsafe { GetFirmwareType(&mut kind) } != 0;
    match (read, kind) {
        (true, 1) => "Legacy BIOS",
        (true, 2) => "UEFI",
        _ => "",
    }
    .to_owned()
}

fn secure_boot() -> String {
    match system::reg_dword(Hive::LocalMachine, SECURE_BOOT, "UEFISecureBootEnabled") {
        // The key is missing where the firmware has no Secure Boot, as with a legacy BIOS.
        None => "Not supported",
        Some(0) => "Off",
        Some(_) => "On",
    }
    .to_owned()
}

/// TPM_DEVICE_INFO.
#[repr(C)]
#[derive(Default)]
struct TpmInfo {
    struct_version: u32,
    tpm_version: u32,
    interface_type: u32,
    revision: u32,
}

/// The TPM's version, which Windows 11 needs to be 2.0. TPM Base Services tell anyone; Get-Tpm needs an
/// administrator.
fn tpm() -> String {
    let mut info = TpmInfo::default();
    let result = unsafe { Tbsi_GetDeviceInfo(size_of::<TpmInfo>() as u32, (&raw mut info).cast()) };
    match (result, info.tpm_version) {
        (TPM_NOT_FOUND, _) => "Not found, or turned off in the BIOS",
        (0, 1) => "1.2",
        (0, 2) => "2.0",
        _ => "",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::Vendor;
    use crate::hardware::{BUS_NVME, BUS_USB, Volume};

    const GB: u64 = 1024 * 1024 * 1024;

    fn bios(entries: &[(&'static str, &str)]) -> Values {
        entries.iter().map(|(name, value)| (*name, (*value).to_owned())).collect()
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn windows_11_is_told_apart_by_its_build_number() {
        // The registry still says "Windows 10" on Windows 11.
        assert_eq!(windows_name("Windows 10 Pro", 26300), "Windows 11 Pro");
        assert_eq!(windows_name("Windows 10 Home", 19045), "Windows 10 Home");
        assert_eq!(windows_name("Windows Server 2025 Datacenter", 26100), "Windows Server 2025 Datacenter");
    }

    #[test]
    fn model_names_maker_and_model_or_the_mainboard_of_a_pc_built_from_parts() {
        for (entries, expected) in [
            (vec![("SystemManufacturer", "Dell Inc."), ("SystemProductName", "XPS 15 9530")], "Dell Inc. XPS 15 9530"),
            (
                vec![("SystemManufacturer", "HP"), ("SystemProductName", "HP EliteBook 840 G8 Notebook PC")],
                "HP EliteBook 840 G8 Notebook PC",
            ),
            (
                vec![
                    ("SystemManufacturer", "LENOVO"),
                    ("SystemProductName", "21HMCTO1WW"),
                    ("SystemVersion", "ThinkPad X1 Carbon Gen 11"),
                ],
                "LENOVO ThinkPad X1 Carbon Gen 11",
            ),
            (
                vec![
                    ("SystemManufacturer", "System manufacturer"),
                    ("SystemProductName", "System Product Name"),
                    ("BaseBoardManufacturer", "ASUSTeK COMPUTER INC."),
                    ("BaseBoardProduct", "ROG STRIX B550-F GAMING"),
                ],
                "ASUSTeK COMPUTER INC. ROG STRIX B550-F GAMING",
            ),
            (vec![], ""),
        ] {
            assert_eq!(model(&bios(&entries)), expected);
        }
    }

    #[test]
    fn the_motherboard_is_named_only_where_the_model_is_not_it() {
        let dell = bios(&[
            ("SystemManufacturer", "Dell Inc."),
            ("SystemProductName", "XPS 15 9530"),
            ("BaseBoardManufacturer", "Dell Inc."),
            ("BaseBoardProduct", "0RH1JY"),
        ]);
        assert_eq!(motherboard(&dell), "Dell Inc. 0RH1JY");
        let parts = bios(&[("BaseBoardManufacturer", "ASUSTeK COMPUTER INC."), ("BaseBoardProduct", "PRIME Z790-P")]);
        assert_eq!(motherboard(&parts), "");
    }

    #[test]
    fn bios_version_comes_with_its_release_date_and_age() {
        let today = day(2026, 11, 3);
        assert_eq!(
            bios_version(&bios(&[("BIOSVersion", "22.02"), ("BIOSReleaseDate", "09/25/2025")]), today),
            "22.02 · Sep 25, 2025 (13 months old)"
        );
        assert_eq!(bios_version(&bios(&[("BIOSVersion", "1.12.0"), ("BIOSReleaseDate", "sometime")]), today), "1.12.0 · sometime");
        assert_eq!(bios_version(&bios(&[("BIOSVersion", "F12")]), today), "F12");
    }

    #[test]
    fn age_is_given_in_months_then_years() {
        for (from, text) in [
            (day(2026, 10, 20), "less than a month old"),
            (day(2026, 10, 3), "1 month old"),
            (day(2025, 9, 25), "13 months old"),
            (day(2023, 1, 1), "3 years old"),
        ] {
            assert_eq!(age(from, day(2026, 11, 3)), text);
        }
    }

    #[test]
    fn brand_pcs_get_their_makers_support_site_and_others_a_search_for_their_model() {
        assert!(bios_page(&bios(&[("SystemManufacturer", "Dell Inc."), ("SystemProductName", "XPS 15")])).starts_with("https://www.dell.com/"));
        assert!(bios_page(&bios(&[("SystemManufacturer", "HP"), ("SystemProductName", "HP EliteBook")])).starts_with("https://support.hp.com/"));
        assert_eq!(bios_page(&bios(&[("SystemManufacturer", "Microsoft Corporation")])), "ms-settings:windowsupdate");
        assert_eq!(
            bios_page(&bios(&[("SystemManufacturer", "ASRock"), ("SystemProductName", "Z690 PG Velocita")])),
            "https://duckduckgo.com/?q=ASRock+Z690+PG+Velocita+BIOS+update"
        );
        assert_eq!(quote_plus("a&b/c ä"), "a%26b%2Fc+%C3%A4");
    }

    #[test]
    fn who_manages_the_pc() {
        for (domain, tenant, text) in [
            (None, None, "Not managed by an organization"),
            (Some("CONTOSO"), None, "CONTOSO (Active Directory)"),
            (None, Some("Contoso"), "Contoso (Microsoft Entra ID)"),
            (None, Some(""), "Microsoft Entra ID"),
            (Some("CONTOSO"), Some("Contoso"), "Contoso (Microsoft Entra ID)\nCONTOSO (Active Directory)"),
        ] {
            assert_eq!(management(domain, tenant), text);
        }
    }

    #[test]
    fn installed_on_is_the_first_installation_not_the_last_upgrade() {
        // Windows 10 installed in 2021, upgraded to Windows 11 in 2023 and to a new version in 2026.
        let (first, upgrade, current) = (1_620_000_000, 1_690_000_000, 1_771_623_207);
        assert_eq!(first_installed(Some(current), &[Some(upgrade), Some(first)]), Some(first));
        assert_eq!(first_installed(Some(current), &[]), Some(current));
        assert_eq!(first_installed(Some(current), &[None, Some(0)]), Some(current));
        assert_eq!(first_installed(None, &[]), None);
    }

    #[test]
    fn the_clipboard_gets_a_line_per_shown_row() {
        let sections = vec![
            Section::new(
                "Device",
                vec![
                    Row::always("Name", "PC"),
                    Row::new("Processor", "Intel\n12 cores", Show::Summary),
                ],
            ),
            Section::new("Processor", vec![Row::details("Cores", "12 (8 performance + 4 efficient)")]),
            Section { title: "Firmware", rows: vec![Row::always("TPM", "2.0")], action: Some(("Find BIOS updates", "https://example.com".to_owned())) },
        ];
        assert_eq!(as_text(&sections, false), "Device\nName: PC\nProcessor: Intel, 12 cores\n\nFirmware\nTPM: 2.0");
        assert_eq!(
            as_text(&sections, true),
            "Device\nName: PC\n\nProcessor\nCores: 12 (8 performance + 4 efficient)\n\nFirmware\nTPM: 2.0"
        );
    }

    #[test]
    fn processor_details_name_the_kinds_of_cores_and_the_caches() {
        assert_eq!(processor_summary("Intel Core i7", 12, 20), "Intel Core i7\n12 cores, 20 threads");
        assert_eq!(processor_summary("AMD Ryzen 5", 6, 6), "AMD Ryzen 5\n6 cores");
        assert_eq!(processor_summary("Name", 0, 8), "Name");
        assert_eq!(core_text(&[8, 4]), "12 (8 performance + 4 efficient)");
        assert_eq!(core_text(&[6, 8, 2]), "16 (6 performance + 8 efficient + 2 low-power efficient)");
        assert_eq!(core_text(&[8]), "8");
        assert_eq!(core_text(&[]), "");
        assert_eq!(cache_text(&[1024 * 1024, 12 * 1024 * 1024, 25 * 1024 * 1024]), "L1 1 MB · L2 12 MB · L3 25 MB");
        assert_eq!(cache_text(&[640 * 1024, 1280 * 1024, 0]), "L1 640 KB · L2 1.25 MB");
        let topology = Topology { cores: vec![8, 4], caches: [1024 * 1024, 0, 0], packages: 1 };
        let rows = processor_rows("Intel Core i7-12700K", &topology, 20, Some(3610));
        let shown: Vec<(&str, &str)> = rows.iter().map(|row| (row.label.as_str(), row.value.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("Name", "Intel Core i7-12700K"),
                ("Cores", "12 (8 performance + 4 efficient)"),
                ("Threads", "20"),
                ("Base speed", "3.61 GHz"),
                ("Cache", "L1 1 MB"),
            ]
        );
        assert!(rows.iter().all(|row| row.show == Show::Details));
    }

    fn module(slot: &str, maker: Option<&str>, part: Option<&str>, soldered: bool) -> MemoryModule {
        MemoryModule {
            slot: slot.to_owned(),
            size: 16 * GB,
            kind: Some("DDR5"),
            speed: Some(6000),
            maker: maker.map(str::to_owned),
            part: part.map(str::to_owned),
            soldered,
        }
    }

    #[test]
    fn memory_details_tell_the_modules_apart() {
        let memory = Memory {
            modules: vec![
                module("DIMM A2", Some("Kingston"), Some("KF560C36-16"), false),
                module("DIMM B2", None, None, false),
            ],
            slots: 4,
        };
        let rows = memory_rows(Some(32 * GB), &memory);
        let shown: Vec<(&str, &str)> = rows.iter().map(|row| (row.label.as_str(), row.value.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("Installed", "32 GB"),
                ("Slots", "2 of 4 used"),
                ("Module 1", "Kingston KF560C36-16\n16 GB DDR5-6000 · slot DIMM A2"),
                ("Module 2", "16 GB DDR5-6000 · slot DIMM B2"),
            ]
        );
        let laptop = Memory {
            modules: vec![module("A", Some("Micron"), None, true), module("B", Some("Micron"), None, true)],
            slots: 8,
        };
        let rows = memory_rows(None, &laptop);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].value, "Micron\n32 GB DDR5-6000, soldered to the board");
        assert!(memory_rows(Some(GB), &Memory::default()).is_empty());
    }

    #[test]
    fn graphics_details_show_video_memory_and_the_driver() {
        let adapters = [
            Adapter {
                vendor: Some(Vendor::Nvidia),
                name: "NVIDIA GeForce RTX 3080".to_owned(),
                driver_version: "32.0.15.9186".to_owned(),
                driver_date: Some(day(2025, 9, 25)),
                driver_provider: "NVIDIA".to_owned(),
                memory: Some(10 * GB),
            },
            Adapter {
                vendor: None,
                name: "Virtual Display".to_owned(),
                driver_version: String::new(),
                driver_date: None,
                driver_provider: String::new(),
                memory: None,
            },
        ];
        let rows = graphics_rows(&adapters);
        assert_eq!(rows[0].label, "GPU 1");
        assert_eq!(rows[0].value, "NVIDIA GeForce RTX 3080\n10 GB video memory\nDriver 591.86 from Sep 25, 2025");
        assert_eq!(rows[1].label, "GPU 2");
        assert_eq!(rows[1].value, "Virtual Display (no driver)");
        assert_eq!(graphics_rows(&adapters[..1])[0].label, "GPU");
        // What an RTX 5070 Ti's driver reports, a 1.5 GB card's and an integrated chip's.
        assert_eq!(video_memory(0x3_FAF0_0000), "16 GB");
        assert_eq!(video_memory(3 * GB / 2), "1.5 GB");
        assert_eq!(video_memory(128 * 1024 * 1024), "128 MB");
    }

    #[test]
    fn drive_details_show_each_disk_with_its_drive_letters() {
        let volume = |path: &str, label: &str, space: Option<(u64, u64)>| Volume {
            paths: if path.is_empty() { Vec::new() } else { vec![path.to_owned()] },
            label: label.to_owned(),
            file_system: "NTFS".to_owned(),
            space,
        };
        let disks = [
            Disk {
                number: 0,
                model: "Samsung SSD 990 PRO 2TB".to_owned(),
                serial: String::new(),
                size: 2_000_398_934_016,
                bus: BUS_NVME,
                removable_media: false,
                solid_state: Some(true),
                volumes: vec![
                    volume(r"D:\", "Studio", Some((111 * GB, 931 * GB))),
                    volume("", "", None),
                    volume(r"C:\", "", Some((400 * GB, 931 * GB))),
                ],
            },
            Disk {
                number: 2,
                model: String::new(),
                serial: String::new(),
                size: 64 * GB,
                bus: BUS_USB,
                removable_media: true,
                solid_state: None,
                volumes: Vec::new(),
            },
        ];
        let rows = drive_rows(&disks);
        assert_eq!(rows[0].label, "Disk 0");
        assert_eq!(
            rows[0].value,
            "Samsung SSD 990 PRO 2TB\n1.82 TB NVMe SSD\nC: 400 GB free of 931 GB\nD: Studio, 111 GB free of 931 GB"
        );
        assert_eq!(rows[1].label, "Disk 2");
        assert_eq!(rows[1].value, "64 GB USB flash drive\nNo drive letter");
    }

    #[test]
    fn reading_this_pc_leaves_no_row_empty() {
        let sections = gather("Up to date", day(2026, 11, 8));
        assert_eq!(sections.first().map(|section| section.title), Some("Device"));
        assert!(sections.iter().all(|section| !section.rows.is_empty() && section.rows.iter().all(|row| !row.value.is_empty())));
        let windows = sections.iter().find(|section| section.title == "Windows").unwrap();
        assert!(windows.rows.iter().any(|row| row.label == "Updates" && row.value == "Up to date"));
        assert!(windows.rows.iter().any(|row| row.label == "Managed by"));
        let shown = |details| sections.iter().filter(|section| section.is_visible(details)).map(|section| section.title).collect::<Vec<_>>();
        // The short view is what it always was; the details replace its hardware rows with sections of their own.
        assert!(!shown(false).contains(&"Drives"), "{:?}", shown(false));
        assert!(shown(true).contains(&"Drives"), "{:?}", shown(true));
        assert!(as_text(&sections, true).contains("Disk "));
        assert!(as_text(&sections, false).contains("System drive: "));
    }

    #[test]
    fn restart_pending_is_read_without_error() {
        let _ = restart_pending();
    }
}
