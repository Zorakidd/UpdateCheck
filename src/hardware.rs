//! The parts inside the PC as the firmware and Windows describe them: the memory modules, the processor's
//! cores and caches, and the drives with their volumes. All of it is readable without administrator rights.

use std::collections::{BTreeMap, HashMap};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose,
    GetVolumeInformationW, GetVolumePathNamesForVolumeNameW, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY};
use windows_sys::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GetSystemFirmwareTable, RSMB, RelationAll,
};
use windows_sys::Win32::System::Threading::{ALL_PROCESSOR_GROUPS, GetActiveProcessorCount};

use crate::system::{self, from_wide, wide};

const MB: u64 = 1024 * 1024;

// --- Memory ---------------------------------------------------------------------------------------------

/// One memory module, from the firmware's SMBIOS tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryModule {
    /// Where it sits, as the board labels it, such as "DIMM A2"; empty if the firmware doesn't say.
    pub slot: String,
    pub size: u64,
    /// "DDR5", "LPDDR4" and so on.
    pub kind: Option<&'static str>,
    /// The speed it runs at in MT/s, or else the speed it is rated for.
    pub speed: Option<u32>,
    pub maker: Option<String>,
    pub part: Option<String>,
    /// Soldered to the board, rather than a stick that can be swapped.
    pub soldered: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub modules: Vec<MemoryModule>,
    /// Sockets for modules, used or not; 0 if the firmware doesn't list them.
    pub slots: usize,
}

pub fn memory() -> Memory {
    smbios().map(|table| parse_memory(&table)).unwrap_or_default()
}

/// The SMBIOS tables as Windows hands them out: a header of 8 bytes, then the structures.
fn smbios() -> Option<Vec<u8>> {
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, null_mut(), 0) };
    if size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    let written = unsafe { GetSystemFirmwareTable(RSMB, 0, buffer.as_mut_ptr(), size) };
    if written == 0 || written > size {
        return None;
    }
    buffer.truncate(written as usize);
    Some(buffer)
}

/// One SMBIOS structure: its formatted part and the strings that follow it.
struct Structure<'a> {
    kind: u8,
    handle: u16,
    data: &'a [u8],
    strings: Vec<&'a [u8]>,
}

impl Structure<'_> {
    fn byte(&self, offset: usize) -> Option<u8> {
        self.data.get(offset).copied()
    }

    fn word(&self, offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.data.get(offset..offset + 2)?.try_into().ok()?))
    }

    fn dword(&self, offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.data.get(offset..offset + 4)?.try_into().ok()?))
    }

    /// The string a field points to; fields count the strings from 1, and 0 means none.
    fn string(&self, offset: usize) -> Option<String> {
        let index = usize::from(self.byte(offset)?).checked_sub(1)?;
        let text = String::from_utf8_lossy(self.strings.get(index)?).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }
}

fn structures(table: &[u8]) -> Vec<Structure<'_>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while offset + 4 <= table.len() {
        let kind = table[offset];
        let length = usize::from(table[offset + 1]);
        if length < 4 || offset + length > table.len() {
            break;
        }
        let handle = u16::from_le_bytes([table[offset + 2], table[offset + 3]]);
        let data = &table[offset..offset + length];
        // Each string ends in a NUL and the set in a second one; a structure without strings has just the two.
        let mut position = offset + length;
        let mut strings = Vec::new();
        if table.get(position) == Some(&0) {
            position += 2;
        } else {
            loop {
                let start = position;
                while position < table.len() && table[position] != 0 {
                    position += 1;
                }
                strings.push(&table[start..position]);
                position += 1;
                if position >= table.len() || table[position] == 0 {
                    position += 1;
                    break;
                }
            }
        }
        found.push(Structure { kind, handle, data, strings });
        // Type 127 marks the end of the table.
        if kind == 127 {
            break;
        }
        offset = position;
    }
    found
}

fn parse_memory(raw: &[u8]) -> Memory {
    let Some(length) = raw.get(4..8).and_then(|bytes| bytes.try_into().ok()).map(u32::from_le_bytes) else {
        return Memory::default();
    };
    let table = &raw[8..raw.len().min(8 + length as usize)];
    let all = structures(table);
    // Memory arrays (type 16) that hold the system's memory, as opposed to, say, a cache's.
    let system_arrays: Vec<u16> =
        all.iter().filter(|structure| structure.kind == 16 && structure.byte(0x05) == Some(3)).map(|s| s.handle).collect();
    let mut memory = Memory::default();
    for device in all.iter().filter(|structure| structure.kind == 17) {
        if !system_arrays.is_empty() && device.word(0x04).is_some_and(|array| !system_arrays.contains(&array)) {
            continue;
        }
        memory.slots += 1;
        let size = match device.word(0x0C) {
            // An empty socket, or a size the firmware doesn't know.
            None | Some(0) | Some(0xFFFF) => 0,
            Some(0x7FFF) => device.dword(0x1C).map_or(0, |megabytes| u64::from(megabytes & 0x7FFF_FFFF) * MB),
            Some(word) if word & 0x8000 != 0 => u64::from(word & 0x7FFF) * 1024,
            Some(word) => u64::from(word) * MB,
        };
        if size == 0 {
            continue;
        }
        let code = device.byte(0x12).unwrap_or(0);
        let speed = |word_at: usize, dword_at: usize| match device.word(word_at) {
            Some(0xFFFF) => device.dword(dword_at).map(|speed| speed & 0x7FFF_FFFF).filter(|speed| *speed > 0),
            None | Some(0) => None,
            Some(speed) => Some(u32::from(speed)),
        };
        let form = device.byte(0x0E).unwrap_or(0);
        let low_power = matches!(code, 0x1B..=0x1E | 0x23);
        memory.modules.push(MemoryModule {
            slot: slot_name(device.string(0x10), device.string(0x11)),
            size,
            kind: memory_kind(code),
            // The configured speed is what it runs at; the other is what it is rated for.
            speed: speed(0x20, 0x58).or_else(|| speed(0x15, 0x54)),
            maker: device.string(0x17).and_then(|text| memory_maker(&text)),
            part: device.string(0x1A).and_then(|text| part_number(&text)),
            // A chip, a row of chips or a die is on the board; low-power memory is, unless it is a CAMM module.
            soldered: matches!(form, 0x05 | 0x0B | 0x10) || (low_power && form != 0x11),
        });
    }
    memory
}

/// The name of a socket from its device and bank locators. AMD boards name the channel in the bank, as in
/// "P0 CHANNEL A" with "DIMM 1", and others a bare "BANK 0", which adds nothing.
fn slot_name(device: Option<String>, bank: Option<String>) -> String {
    let bank = bank.filter(|bank| {
        let squashed = squash(bank);
        !squashed.strip_prefix("bank").is_some_and(|number| number.bytes().all(|byte| byte.is_ascii_digit()))
    });
    match (bank, device) {
        (Some(bank), Some(device)) if !device.to_lowercase().contains(&bank.to_lowercase()) => format!("{bank} {device}"),
        (_, Some(device)) => device,
        (Some(bank), None) => bank,
        (None, None) => String::new(),
    }
}

fn memory_kind(code: u8) -> Option<&'static str> {
    Some(match code {
        0x0F => "SDRAM",
        0x12 => "DDR",
        0x13 | 0x14 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => return None,
    })
}

/// Makers by their JEDEC bank and code, for firmware that writes the code instead of the name.
const JEDEC: [(usize, u8, &str); 16] = [
    (1, 0xCE, "Samsung"),
    (1, 0xAD, "SK hynix"),
    (1, 0x2C, "Micron"),
    (2, 0x98, "Kingston"),
    (2, 0x4F, "Transcend"),
    (2, 0x7A, "Apacer"),
    (2, 0xBA, "PNY"),
    (3, 0x9E, "Corsair"),
    (3, 0xFE, "Elpida"),
    (4, 0x0B, "Nanya"),
    (5, 0xCD, "G.Skill"),
    (5, 0xEF, "TeamGroup"),
    (5, 0xCB, "ADATA"),
    (5, 0x43, "Ramaxel"),
    (6, 0x9B, "Crucial"),
    (6, 0x02, "Patriot"),
];

/// The spellings firmware uses for the usual makers, squashed to lowercase letters and digits.
const MAKERS: [(&str, &str); 19] = [
    ("samsung", "Samsung"),
    ("skhynix", "SK hynix"),
    ("hynix", "SK hynix"),
    ("hynixsemiconductor", "SK hynix"),
    ("micron", "Micron"),
    ("microntechnology", "Micron"),
    ("crucial", "Crucial"),
    ("crucialtechnology", "Crucial"),
    ("kingston", "Kingston"),
    ("kingstontechnology", "Kingston"),
    ("corsair", "Corsair"),
    ("gskill", "G.Skill"),
    ("gskillintl", "G.Skill"),
    ("teamgroup", "TeamGroup"),
    ("teamgroupinc", "TeamGroup"),
    ("adata", "ADATA"),
    ("adatatechnology", "ADATA"),
    ("patriot", "Patriot"),
    ("patriotmemory", "Patriot"),
];

/// What firmware writes into a field it has nothing for.
const PLACEHOLDERS: [&str; 9] =
    ["unknown", "undefined", "notspecified", "none", "empty", "notavailable", "na", "tobefilledbyoem", "defaultstring"];

fn squash(text: &str) -> String {
    text.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

/// The maker of a memory module, by name, from what the firmware wrote: a name, a JEDEC code or a placeholder.
pub fn memory_maker(text: &str) -> Option<String> {
    let text = text.trim();
    let squashed = squash(text);
    if squashed.is_empty()
        || squashed.contains("manufacturer")
        || PLACEHOLDERS.contains(&squashed.as_str())
        || squashed.bytes().all(|byte| byte == b'0' || byte == b'f')
    {
        return None;
    }
    if squashed.len() == text.len() && squashed.len() >= 4 && squashed.len().is_multiple_of(2) && squashed.bytes().all(|b| b.is_ascii_hexdigit())
        && let Some(name) = jedec_maker(&squashed) {
            return Some(name.to_owned());
        }
    let known = MAKERS.iter().find(|(spelling, _)| *spelling == squashed).map(|(_, name)| (*name).to_owned());
    Some(known.unwrap_or_else(|| text.to_owned()))
}

fn jedec_maker(hex: &str) -> Option<&'static str> {
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).filter_map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok()).collect();
    let find = |bank: usize, code: u8| JEDEC.iter().find(|entry| entry.0 == bank && entry.1 == code).map(|entry| entry.2);
    // As the module's SPD has it: the number of banks skipped, with a parity bit, then the code.
    if let [skipped, code, ..] = bytes[..]
        && let Some(name) = find(usize::from(skipped & 0x7F) + 1, code) {
            return Some(name);
        }
    // The older form: a continuation code 7F for each bank skipped, then the code.
    let skipped = bytes.iter().take_while(|byte| **byte == 0x7F).count();
    bytes.get(skipped).and_then(|code| find(skipped + 1, *code))
}

pub fn part_number(text: &str) -> Option<String> {
    let text = text.trim();
    let squashed = squash(text);
    if squashed.is_empty()
        || squashed.contains("partnum")
        || PLACEHOLDERS.contains(&squashed.as_str())
        || squashed.bytes().all(|byte| byte == b'0')
    {
        return None;
    }
    Some(text.to_owned())
}

// --- Processor ------------------------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Topology {
    /// How many cores of each kind, the fastest kind first. One entry unless the processor mixes kinds, as
    /// Intel's performance and efficient cores do.
    pub cores: Vec<usize>,
    /// Bytes of level 1, 2 and 3 cache, added up over the processor.
    pub caches: [u64; 3],
    /// Processor packages, which is more than one only in a workstation or server with several sockets.
    pub packages: usize,
}

impl Topology {
    pub fn core_count(&self) -> usize {
        self.cores.iter().sum()
    }
}

pub fn topology() -> Topology {
    let mut length = 0u32;
    unsafe { GetLogicalProcessorInformationEx(RelationAll, null_mut(), &mut length) };
    if length == 0 {
        return Topology::default();
    }
    // The records hold pointer-sized fields, so the buffer is aligned for them.
    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
    if unsafe { GetLogicalProcessorInformationEx(RelationAll, buffer.as_mut_ptr().cast(), &mut length) } == 0 {
        return Topology::default();
    }
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), (length as usize).min(buffer.len() * 8)) };
    parse_topology(bytes)
}

fn parse_topology(bytes: &[u8]) -> Topology {
    let read_u32 = |record: &[u8], at: usize| record.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let mut classes: BTreeMap<u8, usize> = BTreeMap::new();
    let mut topology = Topology::default();
    let mut offset = 0;
    while offset + 8 <= bytes.len() {
        let record = &bytes[offset..];
        let relationship = read_u32(record, 0).unwrap_or(u32::MAX);
        let size = read_u32(record, 4).unwrap_or(0) as usize;
        if size < 8 || offset + size > bytes.len() {
            break;
        }
        let record = &record[..size];
        match relationship {
            // A core; the byte after its flags is its efficiency class, higher for faster cores.
            0 => *classes.entry(record.get(9).copied().unwrap_or(0)).or_default() += 1,
            // A cache: its level, then its size at 12 and its type at 16, where 3 is a trace cache.
            2 => {
                if let (Some(&level), Some(cache_size)) = (record.get(8), read_u32(record, 12))
                    && (1..=3).contains(&level) && read_u32(record, 16) != Some(3) {
                        topology.caches[usize::from(level) - 1] += u64::from(cache_size);
                    }
            }
            3 => topology.packages += 1,
            _ => {}
        }
        offset += size;
    }
    topology.cores = classes.into_values().rev().collect();
    topology
}

/// Logical processors over all processor groups, which a PC with more than 64 has several of.
pub fn threads() -> usize {
    unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) as usize }
}

// --- Drives ---------------------------------------------------------------------------------------------

// How a disk is attached (STORAGE_BUS_TYPE).
pub const BUS_ATA: i32 = 3;
pub const BUS_USB: i32 = 7;
pub const BUS_RAID: i32 = 8;
pub const BUS_SATA: i32 = 11;
pub const BUS_SD: i32 = 12;
pub const BUS_MMC: i32 = 13;
pub const BUS_VIRTUAL: i32 = 14;
pub const BUS_FILE_BACKED_VIRTUAL: i32 = 15;
pub const BUS_SPACES: i32 = 16;
pub const BUS_NVME: i32 = 17;
pub const BUS_UFS: i32 = 19;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    /// Where it is mounted, such as "E:\"; empty for a volume without a drive letter or folder.
    pub paths: Vec<String>,
    pub label: String,
    pub file_system: String,
    /// Free and total bytes, if Windows can read the volume.
    pub space: Option<(u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// The number Disk Management shows, as in "Disk 1".
    pub number: u32,
    pub model: String,
    pub serial: String,
    pub size: u64,
    pub bus: i32,
    /// The medium can be taken out, as with a USB stick or a memory card.
    pub removable_media: bool,
    pub solid_state: Option<bool>,
    pub volumes: Vec<Volume>,
}

impl Disk {
    pub fn kind(&self) -> &'static str {
        let by_medium = |ssd: &'static str, hdd: &'static str, other: &'static str| match self.solid_state {
            Some(true) => ssd,
            Some(false) => hdd,
            None => other,
        };
        match self.bus {
            BUS_NVME => "NVMe SSD",
            BUS_SATA | BUS_ATA => by_medium("SATA SSD", "SATA hard disk", "SATA drive"),
            BUS_USB if self.removable_media => "USB flash drive",
            BUS_USB => by_medium("external USB SSD", "external USB hard disk", "external USB drive"),
            BUS_SD | BUS_MMC => "memory card",
            BUS_VIRTUAL | BUS_FILE_BACKED_VIRTUAL => "virtual disk",
            BUS_SPACES => "storage space",
            BUS_RAID => "RAID array",
            BUS_UFS => "UFS storage",
            _ => by_medium("SSD", "hard disk", "drive"),
        }
    }

    /// The drive letters of its volumes, such as "C:".
    pub fn letters(&self) -> Vec<String> {
        let mut letters: Vec<String> = self
            .volumes
            .iter()
            .flat_map(|volume| &volume.paths)
            .filter(|path| path.len() == 3 && path.ends_with(":\\"))
            .map(|path| path[..2].to_owned())
            .collect();
        letters.sort();
        letters
    }
}

struct Device(HANDLE);

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Open a disk or volume for questions only: no access rights are asked for, so no administrator is needed.
fn open_device(path: &str) -> Option<Device> {
    let path = wide(path);
    let handle = unsafe {
        CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
    };
    (handle != INVALID_HANDLE_VALUE).then_some(Device(handle))
}

fn control(device: &Device, code: u32, input: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            device.0,
            code,
            if input.is_empty() { null() } else { input.as_ptr().cast() },
            input.len() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    (ok != 0).then_some(returned as usize)
}

/// A STORAGE_PROPERTY_QUERY for a standard query of the given property.
fn property_query(property: i32) -> [u8; 12] {
    let mut query = [0u8; 12];
    query[..4].copy_from_slice(&property.to_le_bytes());
    query
}

pub fn disks() -> Vec<Disk> {
    let mut volumes = volumes_by_disk();
    // Disk numbers can have gaps once a drive has been removed, so each possible number is tried.
    (0..64).filter_map(|number| disk(number, &mut volumes)).collect()
}

fn disk(number: u32, volumes: &mut HashMap<u32, Vec<Volume>>) -> Option<Disk> {
    let device = open_device(&format!(r"\\.\PhysicalDrive{number}"))?;
    let mut buffer = vec![0u8; 1024];
    let length = control(&device, IOCTL_STORAGE_QUERY_PROPERTY, &property_query(0), &mut buffer)?;
    let descriptor = &buffer[..length.min(buffer.len())];
    let read_u32 = |at: usize| descriptor.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let text_at = |field: usize| -> String {
        let offset = read_u32(field).unwrap_or(0) as usize;
        if offset == 0 || offset >= descriptor.len() {
            return String::new();
        }
        let end = descriptor[offset..].iter().position(|byte| *byte == 0).map_or(descriptor.len(), |at| offset + at);
        String::from_utf8_lossy(&descriptor[offset..end]).split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let vendor = text_at(12);
    let product = text_at(16);
    // SATA drives behind the SCSI layer report "ATA" as their vendor, and some NVMe drives "NVMe".
    let vendor = if ["ata", "nvme", "scsi"].contains(&vendor.to_lowercase().as_str()) { String::new() } else { vendor };
    let model = if vendor.is_empty() || product.to_lowercase().starts_with(&vendor.to_lowercase()) {
        product
    } else {
        format!("{vendor} {product}")
    };
    let mut geometry = [0u8; 256];
    // DISK_GEOMETRY_EX: the geometry (24 bytes), then the size. A card reader without a card has none.
    let size = control(&device, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut geometry)
        .filter(|returned| *returned >= 32)
        .map_or(0, |_| i64::from_le_bytes(geometry[24..32].try_into().unwrap_or_default()).max(0) as u64);
    if size == 0 {
        return None;
    }
    let mut penalty = [0u8; 12];
    // DEVICE_SEEK_PENALTY_DESCRIPTOR: a drive without a seek penalty is solid state.
    let solid_state = control(&device, IOCTL_STORAGE_QUERY_PROPERTY, &property_query(7), &mut penalty)
        .filter(|returned| *returned >= 9)
        .map(|_| penalty[8] == 0);
    Some(Disk {
        number,
        model,
        serial: text_at(24),
        size,
        bus: read_u32(28).map_or(0, |bus| bus as i32),
        removable_media: descriptor.get(10).is_some_and(|byte| *byte != 0),
        solid_state,
        volumes: volumes.remove(&number).unwrap_or_default(),
    })
}

fn volumes_by_disk() -> HashMap<u32, Vec<Volume>> {
    let mut found: HashMap<u32, Vec<Volume>> = HashMap::new();
    let mut name = [0u16; 260];
    let search = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if search == INVALID_HANDLE_VALUE {
        return found;
    }
    loop {
        // A name such as \\?\Volume{…}\.
        let path = from_wide(&name);
        if let Some(disk) = volume_disk(&path) {
            found.entry(disk).or_default().push(volume(&path));
        }
        if unsafe { FindNextVolumeW(search, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    unsafe { FindVolumeClose(search) };
    found
}

/// The disk a volume is on, unless it spreads over several.
fn volume_disk(path: &str) -> Option<u32> {
    let device = open_device(path.trim_end_matches('\\'))?;
    // VOLUME_DISK_EXTENTS: the number of extents, then the first one, which starts with its disk's number.
    let mut extents = [0u8; 32];
    let returned = control(&device, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut extents)?;
    if returned < 32 || extents[..4] != 1u32.to_le_bytes() {
        return None;
    }
    Some(u32::from_le_bytes(extents[8..12].try_into().ok()?))
}

fn volume(path: &str) -> Volume {
    let path_text = wide(path);
    let mut names = vec![0u16; 1024];
    let mut needed = 0u32;
    let paths = if unsafe {
        GetVolumePathNamesForVolumeNameW(path_text.as_ptr(), names.as_mut_ptr(), names.len() as u32, &mut needed)
    } != 0
    {
        names.split(|unit| *unit == 0).filter(|name| !name.is_empty()).map(String::from_utf16_lossy).collect()
    } else {
        Vec::new()
    };
    let mut label = [0u16; 261];
    let mut file_system = [0u16; 261];
    let read = unsafe {
        GetVolumeInformationW(
            path_text.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            null_mut(),
            null_mut(),
            null_mut(),
            file_system.as_mut_ptr(),
            file_system.len() as u32,
        )
    } != 0;
    Volume {
        paths,
        label: if read { from_wide(&label) } else { String::new() },
        file_system: if read { from_wide(&file_system) } else { String::new() },
        space: if read { system::disk_space(path) } else { None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A structure as the firmware lays it out: type, length, handle, the fields, then the strings.
    fn structure(kind: u8, handle: u16, fields: &[u8], strings: &[&str]) -> Vec<u8> {
        let mut bytes = vec![kind, (4 + fields.len()) as u8];
        bytes.extend(handle.to_le_bytes());
        bytes.extend(fields);
        if strings.is_empty() {
            bytes.extend([0, 0]);
        } else {
            for text in strings {
                bytes.extend(text.as_bytes());
                bytes.push(0);
            }
            bytes.push(0);
        }
        bytes
    }

    /// The fields of a memory device (type 17) after its header, up to the configured speed.
    fn memory_device(array: u16, size: u16, form: u8, kind: u8, speed: u16, configured: u16, extended: u32) -> Vec<u8> {
        let mut fields = vec![0u8; 0x22 - 4];
        let mut put = |at: usize, bytes: &[u8]| fields[at - 4..at - 4 + bytes.len()].copy_from_slice(bytes);
        put(0x04, &array.to_le_bytes());
        put(0x0C, &size.to_le_bytes());
        put(0x0E, &[form]);
        put(0x10, &[1]);
        put(0x11, &[2]);
        put(0x12, &[kind]);
        put(0x15, &speed.to_le_bytes());
        put(0x17, &[3]);
        put(0x1A, &[4]);
        put(0x1C, &extended.to_le_bytes());
        put(0x20, &configured.to_le_bytes());
        fields
    }

    fn table(structures: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = structures.concat();
        let mut raw = vec![0, 3, 4, 0];
        raw.extend((body.len() as u32).to_le_bytes());
        raw.extend(body);
        raw
    }

    #[test]
    fn memory_modules_are_read_from_smbios() {
        // A system memory array with four sockets, and one used by a graphics chip that is to be ignored.
        let mut array = vec![0u8; 0x0F - 4];
        array[0x05 - 4] = 3;
        let mut video = vec![0u8; 0x0F - 4];
        video[0x05 - 4] = 4;
        let raw = table(&[
            structure(16, 0x10, &array, &[]),
            structure(16, 0x11, &video, &[]),
            structure(17, 0x20, &memory_device(0x10, 16384, 0x09, 0x22, 4800, 6000, 0), &["DIMM A2", "BANK 0", "0198", "KF560C36-16    "]),
            structure(17, 0x21, &memory_device(0x10, 0, 0x02, 0x02, 0, 0, 0), &["DIMM A1", "BANK 0", "Unknown", "Unknown"]),
            structure(17, 0x22, &memory_device(0x10, 0x7FFF, 0x09, 0x22, 5600, 0, 32768), &["DIMM B2", "BANK 1", "Kingston", "PartNum3"]),
            structure(17, 0x23, &memory_device(0x10, 0, 0x02, 0x02, 0, 0, 0), &["DIMM B1", "BANK 1", "Unknown", "Unknown"]),
            structure(17, 0x30, &memory_device(0x11, 2048, 0x0B, 0x1A, 3200, 3200, 0), &["VRAM", "", "", ""]),
            structure(127, 0xFFFF, &[], &[]),
        ]);
        let memory = parse_memory(&raw);
        assert_eq!(memory.slots, 4);
        assert_eq!(
            memory.modules,
            vec![
                MemoryModule {
                    slot: "DIMM A2".to_owned(),
                    size: 16 * 1024 * MB,
                    kind: Some("DDR5"),
                    speed: Some(6000),
                    maker: Some("Kingston".to_owned()),
                    part: Some("KF560C36-16".to_owned()),
                    soldered: false,
                },
                MemoryModule {
                    slot: "DIMM B2".to_owned(),
                    size: 32 * 1024 * MB,
                    kind: Some("DDR5"),
                    speed: Some(5600),
                    maker: Some("Kingston".to_owned()),
                    part: None,
                    soldered: false,
                },
            ]
        );
    }

    #[test]
    fn soldered_low_power_memory_is_told_apart() {
        let raw = table(&[structure(
            17,
            1,
            &memory_device(0xFFFE, 8192, 0x0B, 0x23, 6400, 6400, 0),
            &["Controller0-ChannelA", "BANK 0", "Micron Technology", "MT62F2G32D4DS-026 WT:B"],
        )]);
        let memory = parse_memory(&raw);
        assert_eq!(memory.slots, 1);
        let module = &memory.modules[0];
        assert!(module.soldered);
        assert_eq!(module.kind, Some("LPDDR5"));
        assert_eq!(module.maker.as_deref(), Some("Micron"));
    }

    #[test]
    fn sockets_are_named_by_channel_where_the_board_says_it() {
        let name = |device: Option<&str>, bank: Option<&str>| slot_name(device.map(str::to_owned), bank.map(str::to_owned));
        assert_eq!(name(Some("DIMM 1"), Some("P0 CHANNEL A")), "P0 CHANNEL A DIMM 1");
        assert_eq!(name(Some("Controller0-ChannelA-DIMM1"), Some("BANK 0")), "Controller0-ChannelA-DIMM1");
        assert_eq!(name(Some("ChannelA-DIMM0"), Some("ChannelA")), "ChannelA-DIMM0");
        assert_eq!(name(None, Some("BANK 2")), "");
        assert_eq!(name(None, Some("Bank A")), "Bank A");
        assert_eq!(name(None, None), "");
    }

    #[test]
    fn broken_tables_are_survived() {
        assert_eq!(parse_memory(&[]), Memory::default());
        assert_eq!(parse_memory(&[0, 3, 4, 0, 200, 0, 0, 0, 17, 40, 0]), Memory::default());
        let mut raw = table(&[structure(17, 1, &memory_device(1, 4096, 9, 0x1A, 2666, 0, 0), &["A"])]);
        raw.truncate(raw.len() - 1);
        assert_eq!(parse_memory(&raw).modules.len(), 1);
    }

    #[test]
    fn makers_are_named_from_jedec_codes_and_spellings() {
        for (text, maker) in [
            ("80CE", Some("Samsung")),
            ("80AD000080AD", Some("SK hynix")),
            ("859B", Some("Crucial")),
            ("0198", Some("Kingston")),
            ("029E", Some("Corsair")),
            ("04CD", Some("G.Skill")),
            ("CE00000000000000", Some("Samsung")),
            ("7F7F7F7F7F9B0000", Some("Crucial")),
            ("G Skill Intl", Some("G.Skill")),
            ("SK Hynix", Some("SK hynix")),
            ("Crucial Technology", Some("Crucial")),
            ("Unknown", None),
            ("Manufacturer00", None),
            ("0000", None),
            ("", None),
            ("Lexar Co Limited", Some("Lexar Co Limited")),
            ("8A45", Some("8A45")),
        ] {
            assert_eq!(memory_maker(text).as_deref(), maker, "{text}");
        }
        assert_eq!(part_number("  CMK32GX5M2B6000C36  ").as_deref(), Some("CMK32GX5M2B6000C36"));
        for placeholder in ["PartNum0", "ModulePartNumber00", "Not Specified", "0000000000", " "] {
            assert_eq!(part_number(placeholder), None, "{placeholder}");
        }
    }

    fn record(relationship: u32, body: &[u8]) -> Vec<u8> {
        let mut bytes = relationship.to_le_bytes().to_vec();
        bytes.extend((8 + body.len() as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }

    fn core(class: u8) -> Vec<u8> {
        let mut body = vec![0u8; 40];
        body[1] = class;
        record(0, &body)
    }

    fn cache(level: u8, size: u32, kind: u32) -> Vec<u8> {
        let mut body = vec![0u8; 40];
        body[0] = level;
        body[4..8].copy_from_slice(&size.to_le_bytes());
        body[8..12].copy_from_slice(&kind.to_le_bytes());
        record(2, &body)
    }

    #[test]
    fn hybrid_processors_are_split_by_core_kind() {
        let mut bytes = Vec::new();
        for class in [1, 1, 0, 0, 0, 0] {
            bytes.extend(core(class));
        }
        bytes.extend(cache(1, 48 * 1024, 2));
        bytes.extend(cache(1, 32 * 1024, 1));
        bytes.extend(cache(2, 2 * 1024 * 1024, 0));
        bytes.extend(cache(3, 24 * 1024 * 1024, 0));
        bytes.extend(cache(3, 1024, 3));
        bytes.extend(record(3, &[0u8; 40]));
        let topology = parse_topology(&bytes);
        assert_eq!(topology.cores, vec![2, 4]);
        assert_eq!(topology.core_count(), 6);
        assert_eq!(topology.caches, [80 * 1024, 2 * 1024 * 1024, 24 * 1024 * 1024]);
        assert_eq!(topology.packages, 1);
        assert_eq!(parse_topology(&[1, 2, 3]), Topology::default());
    }

    #[test]
    fn this_pc_is_read_without_errors() {
        let topology = topology();
        assert!(topology.core_count() >= 1);
        assert!(threads() >= topology.core_count());
        let memory = memory();
        assert!(memory.modules.iter().all(|module| module.size > 0));
        let disks = disks();
        let system = system::system_drive();
        assert!(
            disks.iter().any(|disk| disk.letters().iter().any(|letter| letter.eq_ignore_ascii_case(&system))),
            "no disk holds {system}: {disks:?}"
        );
    }
}
