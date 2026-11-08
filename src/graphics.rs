//! Graphics cards and their drivers: detection, the latest NVIDIA driver and the vendors' own apps.
//!
//! Only NVIDIA can be asked for its latest driver, through the lookup its driver search page uses. That lookup
//! isn't documented, so anything unexpected counts as a failed check. AMD and Intel have nothing a program can
//! query (Intel's download pages refuse non-browser clients), so for their cards the vendor's own app or
//! driver download page takes over.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use serde_json::Value;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY, SP_DEVINFO_DATA, SPDRP_DEVICEDESC, SPDRP_DRIVER, SPDRP_HARDWAREID,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
};

use crate::applog;
use crate::system::{self, Hive};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Vendor {
    // In the order cards are listed: dedicated graphics usually come first.
    Nvidia,
    Amd,
    Intel,
}

impl Vendor {
    pub fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
        }
    }

    pub fn download_page(self) -> &'static str {
        match self {
            Vendor::Nvidia => "https://www.nvidia.com/en-us/drivers/",
            Vendor::Amd => "https://www.amd.com/en/support/download/drivers.html",
            // Intel Driver & Support Assistant, which finds the right driver for the PC.
            Vendor::Intel => "https://www.intel.com/content/www/us/en/support/detect.html",
        }
    }
}

/// The vendor from a hardware ID such as PCI\VEN_10DE&DEV_2820.
pub fn vendor_of(hardware_id: &str) -> Option<Vendor> {
    let upper = hardware_id.to_ascii_uppercase();
    let start = upper.find("VEN_")? + 4;
    let id = upper.get(start..start + 4)?;
    if !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match id {
        "10DE" => Some(Vendor::Nvidia),
        "1002" => Some(Vendor::Amd),
        "8086" => Some(Vendor::Intel),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Card {
    pub vendor: Vendor,
    pub name: String,
    /// The version Windows reports, such as "32.0.16.1692". Empty when only Windows' basic display driver is
    /// installed.
    pub driver_version: String,
}

impl Card {
    /// The driver version as the vendor names it: NVIDIA calls "32.0.16.1692" "616.92".
    pub fn version(&self) -> String {
        let parts: Vec<&str> = self.driver_version.split('.').collect();
        let joined: String = parts[parts.len().saturating_sub(2)..].concat();
        let digits: String = joined.chars().skip(joined.chars().count().saturating_sub(5)).collect();
        if self.vendor == Vendor::Nvidia && digits.len() == 5 && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return format!("{}.{}", &digits[..3], &digits[3..]);
        }
        self.driver_version.clone()
    }
}

/// A graphics adapter as Windows lists it, whatever its vendor, such as a Qualcomm one in an ARM laptop or a
/// virtual machine's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub vendor: Option<Vendor>,
    pub name: String,
    /// Empty while only Windows' basic display driver stands in for the vendor's.
    pub driver_version: String,
    pub driver_date: Option<NaiveDate>,
    pub driver_provider: String,
    /// The memory the card has for itself. Integrated graphics have little or none and share the PC's.
    pub memory: Option<u64>,
}

impl Adapter {
    pub fn card(&self) -> Option<Card> {
        let vendor = self.vendor?;
        let name = if self.driver_version.is_empty() { format!("{} graphics", vendor.name()) } else { self.name.clone() };
        Some(Card { vendor, name, driver_version: self.driver_version.clone() })
    }

    /// The name with a note when the vendor's driver is missing.
    pub fn label(&self) -> String {
        if self.driver_version.is_empty() { format!("{} (no driver)", self.name) } else { self.name.clone() }
    }
}

struct DeviceList(isize);

impl Drop for DeviceList {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn device_property(devices: isize, info: &SP_DEVINFO_DATA, property: u32) -> String {
    let mut buffer = [0u16; 1024];
    let found = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            devices,
            info,
            property,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
            std::ptr::null_mut(),
        )
    };
    // A list of hardware IDs comes back NUL-separated; the first is the most specific.
    if found != 0 { system::from_wide(&buffer) } else { String::new() }
}

/// The day in a driver's DriverDate, such as "9-25-2025".
fn parse_driver_date(text: &str) -> Option<NaiveDate> {
    let mut parts = text.trim().split('-').map(|part| part.parse::<u32>().ok());
    let (month, day, year) = (parts.next()??, parts.next()??, parts.next()??);
    NaiveDate::from_ymd_opt(year as i32, month, day)
}

/// Every graphics adapter that is present. Takes a few milliseconds.
pub fn adapters() -> Result<Vec<Adapter>, String> {
    let devices = unsafe { SetupDiGetClassDevsW(&GUID_DEVCLASS_DISPLAY, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT) };
    if devices == -1 {
        return Err(system::error_text(system::last_error()));
    }
    let devices = DeviceList(devices);
    let mut found = Vec::new();
    for index in 0.. {
        let mut info: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInfo(devices.0, index, &mut info) } == 0 {
            break;
        }
        let vendor = vendor_of(&device_property(devices.0, &info, SPDRP_HARDWAREID));
        let description = device_property(devices.0, &info, SPDRP_DEVICEDESC);
        let driver_key = device_property(devices.0, &info, SPDRP_DRIVER);
        let key = format!(r"SYSTEM\CurrentControlSet\Control\Class\{driver_key}");
        let read = |value: &str| {
            if driver_key.is_empty() { None } else { system::reg_string(Hive::LocalMachine, &key, value) }
        };
        let provider = read("ProviderName").unwrap_or_default();
        // Windows' basic display driver stands in while the vendor's driver is missing.
        let basic = provider == "Microsoft" && vendor.is_some();
        let name = read("DriverDesc").filter(|_| !basic).unwrap_or_else(|| description.clone());
        if name.is_empty() && vendor.is_none() {
            continue;
        }
        let memory = if driver_key.is_empty() || basic {
            None
        } else {
            system::reg_qword(Hive::LocalMachine, &key, "HardwareInformation.qwMemorySize")
                .or_else(|| {
                    // Older drivers keep it in four bytes, so it tops out at 4 GB.
                    system::reg_binary(Hive::LocalMachine, &key, "HardwareInformation.MemorySize")
                        .filter(|bytes| bytes.len() >= 4)
                        .map(|bytes| u64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])))
                        .or_else(|| system::reg_dword(Hive::LocalMachine, &key, "HardwareInformation.MemorySize").map(u64::from))
                })
                .filter(|bytes| *bytes > 0)
        };
        found.push(Adapter {
            vendor,
            name: if name.is_empty() { format!("{} graphics", vendor.map_or("Unknown", Vendor::name)) } else { name },
            driver_version: if basic { String::new() } else { read("DriverVersion").unwrap_or_default() },
            driver_date: if basic { None } else { read("DriverDate").and_then(|date| parse_driver_date(&date)) },
            driver_provider: provider,
            memory,
        });
    }
    Ok(found)
}

/// The NVIDIA, AMD and Intel graphics cards in this PC.
pub fn detect() -> Result<Vec<Card>, String> {
    let mut cards: Vec<Card> = adapters()?.iter().filter_map(Adapter::card).collect();
    cards.sort_by(|a, b| (a.vendor, &a.name).cmp(&(b.vendor, &b.name)));
    cards.dedup();
    Ok(cards)
}

// --- The latest NVIDIA driver -------------------------------------------------------------------------------

const NVIDIA_PRODUCTS: &str = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3";
/// osID 57 is 64-bit Windows 10 and 11; dch=1 is the driver type those use.
const NVIDIA_DRIVERS: &str = "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php\
?func=DriverManualLookup&osID=57&languageCode=1033&isWHQL=1&dch=1&sort1=0&numberOfResults=1&pfid=";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// The driver's download page.
    pub url: String,
}

fn numbers(version: &str) -> Vec<u64> {
    version
        .split(|char: char| !char.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

pub fn is_newer(release: &Release, card: &Card) -> bool {
    numbers(&release.version) > numbers(&card.version())
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let get = || crate::http::get(url, &[], Duration::from_secs(15), 16 * 1024 * 1024);
    // NVIDIA's driver lookup at times takes seconds to answer, or none at all; a second try mostly gets through.
    let response = get().or_else(|_| get())?;
    if response.status != 200 {
        return Err(format!("NVIDIA answered with HTTP status {}", response.status));
    }
    Ok(response.body)
}

/// The newest driver for the card, or None if NVIDIA's driver search doesn't list the card.
pub fn latest_nvidia_driver(card: &Card) -> Result<Option<Release>, String> {
    let products = download(NVIDIA_PRODUCTS)?;
    let Some(product) = nvidia_product_id(&String::from_utf8_lossy(&products), &card.name) else { return Ok(None) };
    parse_nvidia_driver(&download(&format!("{NVIDIA_DRIVERS}{product}"))?)
}

fn unescape_xml(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            result.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let char = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .map(|hex| u32::from_str_radix(hex, 16).ok())
                .unwrap_or_else(|| entity.strip_prefix('#').and_then(|decimal| decimal.parse().ok()))
                .and_then(char::from_u32),
        };
        match char {
            Some(char) => {
                result.push(char);
                rest = &rest[end + 1..];
            }
            None => {
                result.push('&');
                rest = &rest[1..];
            }
        }
    }
    result.push_str(rest);
    result
}

/// The text of the first `<tag>` element in `xml`.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))?;
    Some(&xml[start..start + end])
}

fn product_key(name: &str) -> String {
    let lower = name.to_lowercase();
    let lower = lower.strip_prefix("nvidia ").unwrap_or(&lower);
    let cleaned = lower.replace("(r)", " ").replace("(tm)", " ").replace(['®', '™'], " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Find a card in NVIDIA's product list, whose names may lack the "NVIDIA" prefix and memory size.
pub fn nvidia_product_id(products_xml: &str, name: &str) -> Option<String> {
    let mut products: HashMap<String, String> = HashMap::new();
    let mut rest = products_xml;
    while let Some(start) = rest.find("<LookupValue") {
        rest = &rest[start + "<LookupValue".len()..];
        // <LookupValues> is the list around them.
        if !rest.starts_with([' ', '>']) {
            continue;
        }
        let Some(end) = rest.find("</LookupValue>") else { break };
        let block = &rest[..end];
        rest = &rest[end..];
        let (Some(product), Some(value)) = (element(block, "Name"), element(block, "Value")) else { continue };
        // Some cards are listed twice, as desktop and notebook products; both get the same driver version.
        products.entry(product_key(&unescape_xml(product))).or_insert_with(|| unescape_xml(value).trim().to_owned());
    }
    let key = product_key(name);
    let without_memory = key.rsplit_once(' ').and_then(|(head, tail)| {
        let digits = tail.strip_suffix("gb")?;
        (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(head)
    });
    [Some(key.as_str()), without_memory]
        .into_iter()
        .flatten()
        .find_map(|key| products.get(key).filter(|value| !value.is_empty()).cloned())
}

/// %XX escapes decoded, as NVIDIA sends its download pages' addresses.
fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = text.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

pub fn parse_nvidia_driver(response: &[u8]) -> Result<Option<Release>, String> {
    let answer: Value = serde_json::from_slice(response).map_err(|_| "NVIDIA's answer isn't the expected JSON".to_owned())?;
    let Some(drivers) = answer.get("IDS").and_then(Value::as_array).filter(|drivers| !drivers.is_empty()) else {
        return Ok(None);
    };
    let info = drivers[0].get("downloadInfo").ok_or("NVIDIA's answer has no downloadInfo")?;
    let version = info.get("Version").and_then(Value::as_str).unwrap_or_default();
    let well_formed = version
        .split_once('.')
        .is_some_and(|(major, minor)| [major, minor].iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())));
    if !well_formed {
        return Err(format!("Unexpected NVIDIA driver version {version:?}"));
    }
    let url = info.get("DetailsURL").and_then(Value::as_str).map(unquote).unwrap_or_default();
    Ok(Some(Release {
        version: version.to_owned(),
        url: if url.starts_with("https://") { url } else { Vendor::Nvidia.download_page().to_owned() },
    }))
}

// --- Lookups in the background ------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverStatus {
    Checking,
    UpToDate(Release),
    UpdateAvailable(Release),
    CheckFailed,
    NotChecked,
    NoDriver,
}

#[derive(Clone)]
enum Lookup {
    Found(Option<Release>),
    Failed,
}

pub type DetectFn = Box<dyn Fn() -> Result<Vec<Card>, String> + Send>;
pub type LookUpFn = Arc<dyn Fn(&Card) -> Result<Option<Release>, String> + Send + Sync>;

/// How long a failed lookup waits before it is tried again, for example once the PC is back online.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The PC's graphics cards and their latest drivers, looked up in background threads.
///
/// `refresh()` detects the cards and starts lookups; `collect()`, called from the window's thread, takes in
/// the finished ones.
pub struct DriverChecks {
    pub cards: Vec<Card>,
    detect: DetectFn,
    look_up: LookUpFn,
    /// Finished lookups by card name. The latest driver for a card doesn't change when its driver is updated.
    lookups: HashMap<String, Lookup>,
    failed_at: HashMap<String, Instant>,
    running: HashSet<String>,
    sender: Sender<(String, Lookup)>,
    receiver: Receiver<(String, Lookup)>,
    notify: Arc<dyn Fn() + Send + Sync>,
    detect_failed: bool,
}

impl DriverChecks {
    pub fn new(detect: DetectFn, look_up: LookUpFn, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (sender, receiver) = channel();
        DriverChecks {
            cards: Vec::new(),
            detect,
            look_up,
            lookups: HashMap::new(),
            failed_at: HashMap::new(),
            running: HashSet::new(),
            sender,
            receiver,
            notify,
            detect_failed: false,
        }
    }

    pub fn standard(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        DriverChecks::new(Box::new(detect), Arc::new(latest_nvidia_driver), notify)
    }

    #[cfg(test)]
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
    }

    /// Detect the cards and look up drivers that haven't been, whose lookup failed a while ago, or all of them
    /// with `recheck`. Returns true if anything shown about the cards changed.
    pub fn refresh(&mut self, recheck: bool) -> bool {
        let cards = match (self.detect)() {
            Ok(cards) => cards,
            Err(error) => {
                if !self.detect_failed {
                    applog::warning("graphics", format!("Couldn't detect the graphics cards: {error}"));
                }
                self.detect_failed = true;
                Vec::new()
            }
        };
        let mut changed = cards != self.cards;
        self.cards = cards;
        for card in self.cards.clone() {
            if card.vendor != Vendor::Nvidia || card.driver_version.is_empty() || self.running.contains(&card.name) {
                continue;
            }
            let lookup = self.lookups.get(&card.name);
            let retry = matches!(lookup, Some(Lookup::Failed))
                && self.failed_at.get(&card.name).is_some_and(|when| when.elapsed() >= RETRY_AFTER);
            if !(recheck || lookup.is_none() || retry) {
                continue;
            }
            self.lookups.remove(&card.name);
            self.running.insert(card.name.clone());
            let (sender, notify, look_up) = (self.sender.clone(), self.notify.clone(), self.look_up.clone());
            let started = std::thread::Builder::new().name("nvidia-driver".into()).spawn(move || {
                let lookup = match look_up(&card) {
                    Ok(release) => Lookup::Found(release),
                    Err(error) => {
                        // NVIDIA's lookup isn't documented and may change: whatever goes wrong, the check failed.
                        applog::warning("graphics", format!("Couldn't look up the latest driver for {}: {error}", card.name));
                        Lookup::Failed
                    }
                };
                let _ = sender.send((card.name, lookup));
                notify();
            });
            if started.is_err() {
                self.running.clear();
            }
            changed = true;
        }
        changed
    }

    /// Take in finished lookups. Returns true if there were any.
    pub fn collect(&mut self) -> bool {
        let mut collected = false;
        while let Ok((name, lookup)) = self.receiver.try_recv() {
            self.running.remove(&name);
            if matches!(lookup, Lookup::Failed) {
                self.failed_at.insert(name.clone(), Instant::now());
            }
            self.lookups.insert(name, lookup);
            collected = true;
        }
        collected
    }

    pub fn status(&self, card: &Card) -> DriverStatus {
        if card.driver_version.is_empty() {
            return DriverStatus::NoDriver;
        }
        if card.vendor != Vendor::Nvidia {
            return DriverStatus::NotChecked;
        }
        match self.lookups.get(&card.name) {
            None => DriverStatus::Checking,
            Some(Lookup::Failed) => DriverStatus::CheckFailed,
            Some(Lookup::Found(None)) => DriverStatus::NotChecked,
            Some(Lookup::Found(Some(release))) if is_newer(release, card) => DriverStatus::UpdateAvailable(release.clone()),
            Some(Lookup::Found(Some(release))) => DriverStatus::UpToDate(release.clone()),
        }
    }
}

// --- The vendors' apps --------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendorApp {
    pub name: &'static str,
    pub exe: PathBuf,
}

impl VendorApp {
    pub fn open(&self) -> Result<(), String> {
        system::shell_execute("open", &self.exe.to_string_lossy(), None, false)
            .map(|_| ())
            .map_err(system::error_text)
    }
}

/// The vendor's graphics app, if it's installed.
pub fn vendor_app(vendor: Vendor) -> Option<VendorApp> {
    let (name, exe) = match vendor {
        Vendor::Nvidia => ("NVIDIA App", r"NVIDIA Corporation\NVIDIA App\CEF\NVIDIA App.exe"),
        Vendor::Amd => ("AMD Software", r"AMD\CNext\CNext\RadeonSoftware.exe"),
        // The successor of Intel Graphics Command Center, which no longer tells about new drivers.
        Vendor::Intel => ("Intel Graphics Software", r"Intel\Intel Graphics Software\IntelGraphicsSoftware.exe"),
    };
    let base = std::env::var_os("ProgramFiles").filter(|path| !path.is_empty()).unwrap_or_else(|| r"C:\Program Files".into());
    let exe = PathBuf::from(base).join(exe);
    exe.is_file().then_some(VendorApp { name, exe })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn rtx_4070_laptop() -> Card {
        Card { vendor: Vendor::Nvidia, name: "NVIDIA GeForce RTX 4070 Laptop GPU".into(), driver_version: "32.0.16.1692".into() }
    }

    /// Asks NVIDIA for real, so it runs only when asked for: cargo test nvidia_online -- --ignored --nocapture
    #[test]
    #[ignore]
    fn nvidia_online() {
        let started = std::time::Instant::now();
        let products = download(NVIDIA_PRODUCTS).map(|body| body.len());
        println!("products: {products:?} after {:?}", started.elapsed());
        let started = std::time::Instant::now();
        println!("latest: {:?} after {:?}", latest_nvidia_driver(&rtx_4070_laptop()), started.elapsed());
    }

    fn iris_xe() -> Card {
        Card { vendor: Vendor::Intel, name: "Intel(R) Iris(R) Xe Graphics".into(), driver_version: "32.0.101.7084".into() }
    }

    fn latest() -> Release {
        Release { version: "616.92".into(), url: "https://www.nvidia.com/en-us/drivers/details/278454/".into() }
    }

    const PRODUCTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><LookupValueSearch><LookupValues>
<LookupValue ParentID="101"><Name>GeForce GTX 1060</Name><Value>817</Value></LookupValue>
<LookupValue ParentID="102"><Name>GeForce GTX 1060</Name><Value>821</Value></LookupValue>
<LookupValue ParentID="127"><Name>NVIDIA GeForce RTX 4070</Name><Value>1015</Value></LookupValue>
<LookupValue ParentID="129"><Name>GeForce RTX 4070 Laptop GPU</Name><Value>1006</Value></LookupValue>
<LookupValue ParentID="130"><Name>Quadro RTX 4000 &amp; more</Name><Value>9</Value></LookupValue>
</LookupValues></LookupValueSearch>"#;

    fn checks(cards: Arc<Mutex<Vec<Card>>>, look_up: impl Fn(&Card) -> Result<Option<Release>, String> + Send + Sync + 'static) -> DriverChecks {
        DriverChecks::new(Box::new(move || Ok(cards.lock().unwrap().clone())), Arc::new(look_up), Arc::new(|| {}))
    }

    fn wait_for(checks: &mut DriverChecks) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while checks.busy() && Instant::now() < deadline {
            checks.collect();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn nvidia_version_comes_from_the_windows_driver_version() {
        for (driver_version, version) in [("32.0.16.1692", "616.92"), ("31.0.15.3623", "536.23"), ("30.0.14.7141", "471.41")] {
            let card = Card { vendor: Vendor::Nvidia, name: "GPU".into(), driver_version: driver_version.into() };
            assert_eq!(card.version(), version);
        }
    }

    #[test]
    fn other_vendors_keep_the_windows_driver_version() {
        assert_eq!(iris_xe().version(), "32.0.101.7084");
    }

    #[test]
    fn vendor_comes_from_the_pci_vendor_id() {
        assert_eq!(vendor_of(r"PCI\VEN_10DE&DEV_2820&SUBSYS_C1CC144D&REV_A1"), Some(Vendor::Nvidia));
        assert_eq!(vendor_of(r"PCI\VEN_1002&DEV_73BF"), Some(Vendor::Amd));
        assert_eq!(vendor_of(r"pci\ven_8086&dev_a7a0"), Some(Vendor::Intel));
        assert_eq!(vendor_of(r"ROOT\BasicDisplay"), None);
        assert_eq!(vendor_of(r"PCI\VEN_10"), None);
    }

    #[test]
    fn detection_returns_cards_and_adapters() {
        let adapters = adapters().unwrap();
        assert!(adapters.iter().all(|adapter| !adapter.name.is_empty()));
        let names: HashSet<String> = adapters.iter().map(|adapter| adapter.name.clone()).collect();
        // The vendors' cards are among them, under the same names.
        for card in detect().unwrap().iter().filter(|card| !card.driver_version.is_empty()) {
            assert!(names.contains(&card.name), "{}", card.name);
        }
    }

    #[test]
    fn driver_dates_are_read_month_first() {
        assert_eq!(parse_driver_date("9-25-2025"), NaiveDate::from_ymd_opt(2025, 9, 25));
        assert_eq!(parse_driver_date("13-1-2025"), None);
        assert_eq!(parse_driver_date(""), None);
    }

    #[test]
    fn cards_are_found_in_nvidias_product_list() {
        for (name, product) in [
            ("NVIDIA GeForce RTX 4070 Laptop GPU", Some("1006")),
            ("NVIDIA GeForce RTX 4070", Some("1015")),
            ("NVIDIA GeForce GTX 1060 6GB", Some("817")),
            ("NVIDIA Quadro RTX 4000 & More", Some("9")),
            ("NVIDIA GeForce RTX 9999", None),
        ] {
            assert_eq!(nvidia_product_id(PRODUCTS, name).as_deref(), product, "{name}");
        }
    }

    #[test]
    fn latest_nvidia_driver_is_read_from_the_lookup() {
        let encoded_url = "https%3A%2F%2Fwww.nvidia.com%2Fen-us%2Fdrivers%2Fdetails%2F278454%2F";
        let response = serde_json::json!({"Success": "1", "IDS": [{"downloadInfo": {"Version": "616.92", "DetailsURL": encoded_url}}]});
        assert_eq!(parse_nvidia_driver(response.to_string().as_bytes()), Ok(Some(latest())));
        assert_eq!(parse_nvidia_driver(br#"{"Success": "0", "IDS": []}"#), Ok(None));
        let odd = serde_json::json!({"IDS": [{"downloadInfo": {"Version": "<html>"}}]});
        assert!(parse_nvidia_driver(odd.to_string().as_bytes()).unwrap_err().contains("Unexpected"));
        assert!(parse_nvidia_driver(b"<html>").is_err());
    }

    #[test]
    fn nvidia_card_is_checked_online_and_others_are_not() {
        let mut checks = checks(Arc::new(Mutex::new(vec![rtx_4070_laptop(), iris_xe()])), |_| Ok(Some(latest())));
        assert!(checks.refresh(false));
        assert_eq!(checks.status(&rtx_4070_laptop()), DriverStatus::Checking);
        assert_eq!(checks.status(&iris_xe()), DriverStatus::NotChecked);
        wait_for(&mut checks);
        assert_eq!(checks.status(&rtx_4070_laptop()), DriverStatus::UpToDate(latest()));
    }

    #[test]
    fn update_is_available_until_the_newer_driver_is_installed() {
        let older = Card { driver_version: "32.0.16.1664".into(), ..rtx_4070_laptop() };
        let installed = Arc::new(Mutex::new(vec![older.clone()]));
        let mut checks = checks(installed.clone(), |_| Ok(Some(latest())));
        checks.refresh(false);
        wait_for(&mut checks);
        assert_eq!(checks.status(&older), DriverStatus::UpdateAvailable(latest()));

        *installed.lock().unwrap() = vec![rtx_4070_laptop()];
        assert!(checks.refresh(false));
        assert!(!checks.busy());
        assert_eq!(checks.status(&rtx_4070_laptop()), DriverStatus::UpToDate(latest()));
    }

    #[test]
    fn failed_lookup_is_retried_after_a_while() {
        let attempts = Arc::new(Mutex::new(0));
        let counter = attempts.clone();
        let mut checks = checks(Arc::new(Mutex::new(vec![rtx_4070_laptop()])), move |_| {
            *counter.lock().unwrap() += 1;
            Err("offline".into())
        });
        checks.refresh(false);
        wait_for(&mut checks);
        assert_eq!(checks.status(&rtx_4070_laptop()), DriverStatus::CheckFailed);

        checks.refresh(false);
        assert_eq!(*attempts.lock().unwrap(), 1);

        checks.failed_at.insert(rtx_4070_laptop().name, Instant::now() - RETRY_AFTER);
        checks.refresh(false);
        wait_for(&mut checks);
        assert_eq!(*attempts.lock().unwrap(), 2);
    }

    #[test]
    fn card_without_its_driver_needs_one() {
        let card = Card { vendor: Vendor::Amd, name: "AMD graphics".into(), driver_version: String::new() };
        let mut checks = checks(Arc::new(Mutex::new(vec![card.clone()])), |_| Ok(Some(latest())));
        checks.refresh(false);
        assert!(!checks.busy());
        assert_eq!(checks.status(&card), DriverStatus::NoDriver);
    }

    #[test]
    fn xml_entities_are_decoded() {
        assert_eq!(unescape_xml("A &amp; B &lt;C&gt; &#65;&#x42; & ;"), "A & B <C> AB & ;");
    }
}
