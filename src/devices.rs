//! The devices plugged into this PC or paired with it: displays, keyboards and mice, audio, cameras, game
//! controllers, drives and the rest, one entry for each thing on the desk.
//!
//! Windows files every device it knows in a "container" for the physical thing it belongs to: a webcam's camera
//! and its microphone share one, a keyboard's dozen interfaces share one, and everything inside the PC shares the
//! PC's own. So the list is of containers, named the way Windows names them under Bluetooth & devices, and what
//! each one is follows from the kinds of device in it. Asking takes a few milliseconds, needs no administrator
//! rights and touches no drive.

use std::ptr::null;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_PRESENT, CM_Get_Device_ID_List_SizeW, CM_PROB_DISABLED, CM_PROB_FAILED_INSTALL,
    CM_PROB_FAILED_POST_START, CM_PROB_FAILED_START, CM_PROB_NEED_RESTART, CM_PROB_REINSTALL, CM_PROB_UNSIGNED_DRIVER,
    CR_SUCCESS,
};
use windows_sys::Win32::Devices::DeviceQuery::{
    DEV_OBJECT, DEV_OBJECT_TYPE, DEVPROP_FILTER_EXPRESSION, DEVPROP_OPERATOR_EQUALS, DevFreeObjects, DevGetObjects,
    DevObjectTypeDevice, DevObjectTypeDeviceContainer, DevQueryFlagLocalize,
};
use windows_sys::Win32::Devices::Properties::{
    DEVPKEY_Device_Class, DEVPKEY_Device_CompatibleIds, DEVPKEY_Device_ContainerId, DEVPKEY_Device_IsPresent,
    DEVPKEY_Device_ProblemCode, DEVPKEY_DeviceContainer_FriendlyName, DEVPKEY_DeviceContainer_IsConnected,
    DEVPKEY_DeviceContainer_IsLocalMachine, DEVPKEY_DeviceContainer_ModelName, DEVPROP_BOOLEAN, DEVPROP_STORE_SYSTEM,
    DEVPROP_TRUE, DEVPROP_TYPE_BOOLEAN, DEVPROP_TYPE_GUID, DEVPROP_TYPE_STRING, DEVPROP_TYPE_STRING_LIST,
    DEVPROP_TYPE_UINT32, DEVPROPCOMPKEY, DEVPROPERTY,
};
use windows_sys::Win32::Foundation::DEVPROPKEY;
use windows_sys::core::GUID;

/// What a device is, in the order the page lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Display,
    Input,
    Audio,
    Camera,
    Controller,
    Storage,
    Printer,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 8] =
        [Kind::Display, Kind::Input, Kind::Audio, Kind::Camera, Kind::Controller, Kind::Storage, Kind::Printer, Kind::Other];

    /// The heading of the kind's card on the page.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Display => "Displays",
            Kind::Input => "Keyboards and mice",
            Kind::Audio => "Audio",
            Kind::Camera => "Cameras",
            Kind::Controller => "Game controllers",
            Kind::Storage => "Drives and phones",
            Kind::Printer => "Printers and scanners",
            Kind::Other => "Other devices",
        }
    }
}

/// How a device is connected, where that can be told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Usb,
    Bluetooth,
}

impl Connection {
    pub fn name(self) -> &'static str {
        match self {
            Connection::Usb => "USB",
            Connection::Bluetooth => "Bluetooth",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: Kind,
    pub connection: Option<Connection>,
    /// What is wrong with it, as Device Manager would say, if anything.
    pub problem: Option<String>,
}

/// A physical thing, as Windows knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Container {
    /// A GUID in braces, as `{09a767d4-b5c9-5032-bef1-ff372c662ad8}`.
    pub id: String,
    pub name: String,
    pub model: String,
    pub connected: bool,
    /// The PC itself, which holds everything built into it.
    pub local: bool,
}

/// One of the devices a container is made of, such as a keyboard's media keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    /// Its device instance ID, as `USB\VID_1532&PID_0E03\5&2A6EC909&0&6`.
    pub id: String,
    pub container: String,
    /// Its device setup class, as `Keyboard`: the same in every language of Windows.
    pub class: String,
    pub compatible: Vec<String>,
    /// Device Manager's problem code, 0 when it works.
    pub problem: u32,
}

/// What the PC's own container is called when its ID is asked for.
const LOCAL_MACHINE: &str = "{00000000-0000-0000-ffff-ffffffffffff}";
/// The classes that tell what a device is, in the order they decide: a webcam has a microphone, a headset may have
/// a button that counts as a keyboard, and a keyboard may have keys that count as a game controller.
const KINDS: [(Kind, &[&str]); 6] = [
    (Kind::Display, &["Monitor"]),
    (Kind::Printer, &["Printer"]),
    (Kind::Camera, &["Camera"]),
    (Kind::Controller, &["XnaComposite", "XboxComposite"]),
    (Kind::Audio, &["AudioEndpoint", "MEDIA"]),
    (Kind::Input, &["Keyboard", "Mouse"]),
];
/// Classes looked at after a game controller's HID usage.
const STORAGE: [&str; 3] = ["DiskDrive", "WPD", "CDROM"];
/// The HID usages of a gamepad and a joystick, which controllers without a class of their own have.
const GAME_USAGES: [&str; 2] = ["HID_DEVICE_UP:0001_U:0005", "HID_DEVICE_UP:0001_U:0004"];
/// Buses that only Windows and apps make devices on: a container of nothing else, such as Microsoft Print to
/// PDF, is no hardware.
const SOFTWARE_BUSES: [&str; 3] = ["SWD", "ROOT", "HTREE"];
const BLUETOOTH_BUSES: [&str; 4] = ["BTHENUM", "BTHLE", "BTHLEDEVICE", "BTHHFENUM"];
const USB_BUSES: [&str; 3] = ["USB", "USBSTOR", "UASPSTOR"];

/// The devices plugged in or paired and connected now, by kind and then by name.
pub fn connected() -> Result<Vec<Device>, String> {
    let containers = containers()?;
    let parts = parts()?;
    Ok(devices(&containers, &parts))
}

/// Changes when a device is plugged in or out, cheaply enough to ask every second: the length of the list of
/// devices Windows has.
pub fn signal() -> u32 {
    let mut length = 0u32;
    let result = unsafe { CM_Get_Device_ID_List_SizeW(&mut length, null(), CM_GETIDLIST_FILTER_PRESENT) };
    if result == CR_SUCCESS { length } else { 0 }
}

/// The devices among `containers` that are plugged in or connected, made of `parts`.
pub fn devices(containers: &[Container], parts: &[Part]) -> Vec<Device> {
    let mut devices: Vec<Device> = containers
        .iter()
        .filter(|container| container.connected && !container.local && !container.id.eq_ignore_ascii_case(LOCAL_MACHINE))
        .filter_map(|container| {
            let parts: Vec<&Part> = parts.iter().filter(|part| part.container.eq_ignore_ascii_case(&container.id)).collect();
            if parts.iter().all(|part| on_bus(part, &SOFTWARE_BUSES)) {
                return None;
            }
            let kind = kind(&parts);
            Some(Device { name: name(container, kind), kind, connection: connection(&parts), problem: problem(&parts) })
        })
        .collect();
    devices.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    devices
}

fn has_class(parts: &[&Part], classes: &[&str]) -> bool {
    parts.iter().any(|part| classes.iter().any(|class| part.class.eq_ignore_ascii_case(class)))
}

/// What a container is, from the classes of its parts.
pub fn kind(parts: &[&Part]) -> Kind {
    // Scanners and older webcams share the imaging class, and a webcam has a microphone.
    if has_class(parts, &["Image"]) && !has_class(parts, &["Printer", "Monitor", "Camera"]) {
        return if has_class(parts, &["AudioEndpoint", "MEDIA"]) { Kind::Camera } else { Kind::Printer };
    }
    if let Some((kind, _)) = KINDS.iter().find(|(_, classes)| has_class(parts, classes)) {
        return *kind;
    }
    if parts.iter().any(|part| part.compatible.iter().any(|id| GAME_USAGES.iter().any(|usage| id.eq_ignore_ascii_case(usage)))) {
        return Kind::Controller;
    }
    if has_class(parts, &STORAGE) { Kind::Storage } else { Kind::Other }
}

/// The container's name, or for a display the model, which is the part of "Generic Monitor (BenQ GL2780)"
/// worth reading.
pub fn name(container: &Container, kind: Kind) -> String {
    let name = container.name.trim();
    let model = container.model.trim();
    if kind == Kind::Display && !model.is_empty() && name.len() > model.len() && name.to_lowercase().contains(&model.to_lowercase()) {
        return model.to_owned();
    }
    [name, model].into_iter().find(|name| !name.is_empty()).unwrap_or("Unnamed device").to_owned()
}

/// Whether a part's instance ID starts with one of `buses`, as `USB\...`.
fn on_bus(part: &Part, buses: &[&str]) -> bool {
    let bus = part.id.split('\\').next().unwrap_or_default();
    buses.iter().any(|name| bus.eq_ignore_ascii_case(name))
}

pub fn connection(parts: &[&Part]) -> Option<Connection> {
    if parts.iter().any(|part| on_bus(part, &BLUETOOTH_BUSES)) {
        Some(Connection::Bluetooth)
    } else if parts.iter().any(|part| on_bus(part, &USB_BUSES)) {
        Some(Connection::Usb)
    } else {
        None
    }
}

/// What is wrong with the first part that has a problem. Software components an app's installer adds don't
/// count, as the device works without them.
pub fn problem(parts: &[&Part]) -> Option<String> {
    let part = parts.iter().find(|part| part.problem != 0 && !part.class.eq_ignore_ascii_case("SoftwareComponent"))?;
    Some(problem_text(part.problem))
}

/// A problem code in words, with the code Device Manager shows, to look it up by.
pub fn problem_text(code: u32) -> String {
    let what = match code {
        CM_PROB_DISABLED => "Turned off in Device Manager",
        CM_PROB_FAILED_INSTALL => "No driver installed",
        CM_PROB_REINSTALL => "Its driver needs to be installed again",
        CM_PROB_NEED_RESTART => "Works after a restart",
        CM_PROB_FAILED_START => "Couldn't start",
        CM_PROB_FAILED_POST_START => "Stopped after it reported a problem",
        CM_PROB_UNSIGNED_DRIVER => "Its driver isn't signed",
        _ => "Not working properly",
    };
    format!("{what} (code {code})")
}

/// The devices as plain text for the clipboard, by kind as the page shows them: "Razer Kiyo (USB)", and what
/// is wrong with one after a colon.
pub fn as_text(devices: &[Device]) -> String {
    Kind::ALL
        .iter()
        .filter(|kind| devices.iter().any(|device| device.kind == **kind))
        .map(|kind| {
            let mut lines = vec![kind.title().to_owned()];
            lines.extend(devices.iter().filter(|device| device.kind == *kind).map(line));
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One device as plain text: "Razer Kiyo (USB)", and what is wrong with it after a colon.
pub fn line(device: &Device) -> String {
    let mut line = device.name.clone();
    if let Some(connection) = device.connection {
        line.push_str(&format!(" ({})", connection.name()));
    }
    if let Some(problem) = &device.problem {
        line.push_str(&format!(": {problem}"));
    }
    line
}

// --- Asking Windows ---------------------------------------------------------------------------------------------

fn containers() -> Result<Vec<Container>, String> {
    let keys = [
        DEVPKEY_DeviceContainer_FriendlyName,
        DEVPKEY_DeviceContainer_ModelName,
        DEVPKEY_DeviceContainer_IsConnected,
        DEVPKEY_DeviceContainer_IsLocalMachine,
    ];
    let objects = Objects::get(DevObjectTypeDeviceContainer, &keys, false)?;
    Ok(objects
        .iter()
        .map(|object| Container {
            id: object.id(),
            name: object.string(&DEVPKEY_DeviceContainer_FriendlyName),
            model: object.string(&DEVPKEY_DeviceContainer_ModelName),
            connected: object.boolean(&DEVPKEY_DeviceContainer_IsConnected),
            local: object.boolean(&DEVPKEY_DeviceContainer_IsLocalMachine),
        })
        .collect())
}

fn parts() -> Result<Vec<Part>, String> {
    let keys = [DEVPKEY_Device_ContainerId, DEVPKEY_Device_Class, DEVPKEY_Device_CompatibleIds, DEVPKEY_Device_ProblemCode];
    let objects = Objects::get(DevObjectTypeDevice, &keys, true)?;
    Ok(objects
        .iter()
        .filter_map(|object| {
            Some(Part {
                id: object.id(),
                container: object.guid(&DEVPKEY_Device_ContainerId)?,
                class: object.string(&DEVPKEY_Device_Class),
                compatible: object.strings(&DEVPKEY_Device_CompatibleIds),
                problem: object.number(&DEVPKEY_Device_ProblemCode),
            })
        })
        .collect())
}

/// What `DevGetObjects` returned, freed when dropped.
struct Objects {
    count: u32,
    objects: *mut DEV_OBJECT,
}

impl Objects {
    /// Every object of the type with the properties of `keys`, only those present with `present`.
    fn get(kind: DEV_OBJECT_TYPE, keys: &[DEVPROPKEY], present: bool) -> Result<Objects, String> {
        let keys: Vec<DEVPROPCOMPKEY> =
            keys.iter().map(|key| DEVPROPCOMPKEY { Key: *key, Store: DEVPROP_STORE_SYSTEM, LocaleName: null() }).collect();
        let mut yes: DEVPROP_BOOLEAN = DEVPROP_TRUE;
        let filter = DEVPROP_FILTER_EXPRESSION {
            Operator: DEVPROP_OPERATOR_EQUALS,
            Property: DEVPROPERTY {
                CompKey: DEVPROPCOMPKEY { Key: DEVPKEY_Device_IsPresent, Store: DEVPROP_STORE_SYSTEM, LocaleName: null() },
                Type: DEVPROP_TYPE_BOOLEAN,
                BufferSize: size_of::<DEVPROP_BOOLEAN>() as u32,
                Buffer: (&raw mut yes).cast(),
            },
        };
        let (filters, filter) = if present { (1, &raw const filter) } else { (0, null()) };
        let mut objects = Objects { count: 0, objects: std::ptr::null_mut() };
        let result = unsafe {
            DevGetObjects(kind, DevQueryFlagLocalize as u32, keys.len() as u32, keys.as_ptr(), filters, filter, &mut objects.count, &mut objects.objects)
        };
        if result < 0 {
            return Err(format!("Windows didn't list the devices: {}", crate::system::error_text(result as u32)));
        }
        Ok(objects)
    }

    fn iter(&self) -> impl Iterator<Item = Object<'_>> {
        let objects: &[DEV_OBJECT] =
            if self.objects.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(self.objects, self.count as usize) } };
        objects.iter().map(|object| Object { object })
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        if !self.objects.is_null() {
            unsafe { DevFreeObjects(self.count, self.objects) };
        }
    }
}

struct Object<'a> {
    object: &'a DEV_OBJECT,
}

fn same_key(a: &DEVPROPKEY, b: &DEVPROPKEY) -> bool {
    let (a_id, b_id) = (&a.fmtid, &b.fmtid);
    a.pid == b.pid && a_id.data1 == b_id.data1 && a_id.data2 == b_id.data2 && a_id.data3 == b_id.data3 && a_id.data4 == b_id.data4
}

/// A GUID in braces and lower case, as Windows names containers.
pub fn guid_text(guid: &GUID) -> String {
    let d = guid.data4;
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        guid.data1, guid.data2, guid.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

impl Object<'_> {
    fn id(&self) -> String {
        if self.object.pszObjectId.is_null() {
            return String::new();
        }
        let length = (0..).take_while(|&index| unsafe { *self.object.pszObjectId.add(index) } != 0).count();
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(self.object.pszObjectId, length) })
    }

    /// The property's bytes, if it has the type.
    fn value(&self, key: &DEVPROPKEY, kind: u32) -> Option<&[u8]> {
        if self.object.pProperties.is_null() {
            return None;
        }
        let properties = unsafe { std::slice::from_raw_parts(self.object.pProperties, self.object.cPropertyCount as usize) };
        let property = properties.iter().find(|property| same_key(&property.CompKey.Key, key))?;
        if property.Type != kind || property.Buffer.is_null() {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(property.Buffer.cast::<u8>(), property.BufferSize as usize) })
    }

    fn units(&self, key: &DEVPROPKEY, kind: u32) -> Vec<u16> {
        let bytes = self.value(key, kind).unwrap_or_default();
        bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect()
    }

    fn string(&self, key: &DEVPROPKEY) -> String {
        crate::system::from_wide(&self.units(key, DEVPROP_TYPE_STRING))
    }

    fn strings(&self, key: &DEVPROPKEY) -> Vec<String> {
        let units = self.units(key, DEVPROP_TYPE_STRING_LIST);
        units.split(|unit| *unit == 0).filter(|text| !text.is_empty()).map(String::from_utf16_lossy).collect()
    }

    fn boolean(&self, key: &DEVPROPKEY) -> bool {
        self.value(key, DEVPROP_TYPE_BOOLEAN).is_some_and(|bytes| bytes.first().is_some_and(|byte| *byte != 0))
    }

    fn number(&self, key: &DEVPROPKEY) -> u32 {
        let bytes = self.value(key, DEVPROP_TYPE_UINT32).unwrap_or_default();
        bytes.get(..4).map_or(0, |bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn guid(&self, key: &DEVPROPKEY) -> Option<String> {
        let bytes = self.value(key, DEVPROP_TYPE_GUID)?;
        if bytes.len() < size_of::<GUID>() {
            return None;
        }
        let guid = unsafe { bytes.as_ptr().cast::<GUID>().read_unaligned() };
        Some(guid_text(&guid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: &str, container: &str, class: &str) -> Part {
        Part { id: id.to_owned(), container: container.to_owned(), class: class.to_owned(), ..Part::default() }
    }

    fn container(id: &str, name: &str, model: &str) -> Container {
        Container { id: id.to_owned(), name: name.to_owned(), model: model.to_owned(), connected: true, local: false }
    }

    /// Parts as they were on a PC with a webcam, a keyboard, a mouse, a game controller, a display and a USB
    /// hard drive, a few of each.
    fn desk() -> (Vec<Container>, Vec<Part>) {
        let containers = vec![
            Container { local: true, ..container(LOCAL_MACHINE, "DESKTOP-PC", "Z690 PG Velocita") },
            container("{80134d7c-0ea3-11f1-bd54-806e6f6e6963}", "Razer Kiyo", "Razer Kiyo"),
            container("{87c75621-3253-542a-a3cb-16d9e769f89d}", "RAZER HUNTSMAN V3", "RAZER HUNTSMAN V3"),
            container("{d86431b6-a5f6-50f3-9f72-97be7a719893}", "Razer DeathAdder V3 Pro", "Razer DeathAdder V3 Pro"),
            container("{a2ea6a8f-c9a6-11f1-bdd3-806e6f6e6963}", "HID-konformer Gamecontroller", "HID-konformer Gamecontroller"),
            container("{1c2fc2f0-0d8e-5d41-96f5-159b6787794b}", "Generic Monitor (BenQ GL2780)", "BenQ GL2780"),
            container("{09a767d4-b5c9-5032-bef1-ff372c662ad8}", "Elements 2621", "Elements 2621"),
            container("{d397268a-724c-b4ef-d641-c9234ba2948b}", "Microsoft Print to PDF", "Microsoft Print To PDF"),
            Container { connected: false, ..container("{80134d7a-0ea3-11f1-bd54-806e6f6e6963}", "Razer BlackWidow V3 Pro", "") },
            container("{3d55a080-3487-596b-bf80-d5fd70781a20}", "LG ULTRAGEAR", "LG ULTRAGEAR"),
        ];
        let kiyo = "{80134D7C-0EA3-11F1-BD54-806E6F6E6963}";
        let keyboard = "{87c75621-3253-542a-a3cb-16d9e769f89d}";
        let mouse = "{d86431b6-a5f6-50f3-9f72-97be7a719893}";
        let pad = "{a2ea6a8f-c9a6-11f1-bdd3-806e6f6e6963}";
        let monitor = "{1c2fc2f0-0d8e-5d41-96f5-159b6787794b}";
        let drive = "{09a767d4-b5c9-5032-bef1-ff372c662ad8}";
        let gamepad = |id: &str, container: &str| Part {
            compatible: vec!["HID_DEVICE_SYSTEM_GAME".to_owned(), "HID_DEVICE_UP:0001_U:0005".to_owned()],
            ..part(id, container, "HIDClass")
        };
        let parts = vec![
            part(r"PCI\VEN_8086&DEV_7AE0\3&11583659&0&A0", LOCAL_MACHINE, "USB"),
            part(r"SWD\MMDEVAPI\{0.0.1.00000000}.{2e1943d1}", kiyo, "AudioEndpoint"),
            part(r"USB\VID_1532&PID_0E03&MI_00\6&3A21B3C2&0&0000", kiyo, "Camera"),
            part(r"USB\VID_1532&PID_0E03&MI_02\6&3A21B3C2&0&0002", kiyo, "MEDIA"),
            part(r"USB\VID_1532&PID_0E03\5&2A6EC909&0&6", kiyo, "USB"),
            part(r"HID\VID_1532&PID_02E4&MI_01&COL01\7&36", keyboard, "Keyboard"),
            part(r"HID\VID_1532&PID_02E4&MI_02\7&2EA1EB25", keyboard, "Mouse"),
            gamepad(r"HID\VID_1532&PID_02E4&MI_00&COL02\8&D5", keyboard),
            part(r"USB\VID_1532&PID_02E4\SI0000G000000000", keyboard, "USB"),
            part(r"SWD\DRIVERENUM\{A488F33A-B9CF-43C5-8C9}", keyboard, "SoftwareComponent"),
            part(r"HID\VID_1532&PID_00B7&MI_01&COL01\8&5A", mouse, "Keyboard"),
            part(r"HID\VID_1532&PID_00B7&MI_00\8&1E2B9B13", mouse, "Mouse"),
            part(r"USB\VID_1532&PID_00B7\000000000000", mouse, "USB"),
            gamepad(r"HID\VID_046D&PID_0000&IG_00\9&3B46CD5C", pad),
            part(r"USB\VID_045E&PID_028E&REV_0110\01", pad, "XnaComposite"),
            part(r"DISPLAY\BNQ78EC\5&300D53F2&0&UID4352", monitor, "Monitor"),
            part(r"HDAUDIO\FUNC_01&VEN_10DE&DEV_009A\5&1", monitor, "AudioEndpoint"),
            part(r"USBSTOR\DISK&VEN_WD&PROD_ELEMENTS_2621\5758", drive, "DiskDrive"),
            part(r"USB\VID_1058&PID_2621\5758303030303030", drive, "USB"),
            part(r"STORAGE\VOLUME\{601F5B9A-0EB3-11F1-BD6}", drive, "Volume"),
            part(r"SWD\PRINTENUM\{724AE777-C556-4E2B-9D9B}", "{d397268a-724c-b4ef-d641-c9234ba2948b}", "PrintQueue"),
        ];
        (containers, parts)
    }

    #[test]
    fn each_thing_on_the_desk_is_listed_once_with_what_it_is() {
        let (containers, parts) = desk();
        let devices = devices(&containers, &parts);
        let listed: Vec<(Kind, &str, Option<Connection>)> =
            devices.iter().map(|device| (device.kind, device.name.as_str(), device.connection)).collect();
        assert_eq!(
            listed,
            [
                (Kind::Display, "BenQ GL2780", None),
                (Kind::Input, "Razer DeathAdder V3 Pro", Some(Connection::Usb)),
                (Kind::Input, "RAZER HUNTSMAN V3", Some(Connection::Usb)),
                (Kind::Camera, "Razer Kiyo", Some(Connection::Usb)),
                (Kind::Controller, "HID-konformer Gamecontroller", Some(Connection::Usb)),
                (Kind::Storage, "Elements 2621", Some(Connection::Usb)),
            ]
        );
    }

    #[test]
    fn the_pc_itself_software_and_what_is_unplugged_are_left_out() {
        let (containers, parts) = desk();
        let names: Vec<String> = devices(&containers, &parts).into_iter().map(|device| device.name).collect();
        // The LG display's container is connected but has no device present, as after it was switched off.
        for gone in ["DESKTOP-PC", "Microsoft Print to PDF", "Razer BlackWidow V3 Pro", "LG ULTRAGEAR"] {
            assert!(!names.iter().any(|name| name == gone), "{gone} is listed");
        }
    }

    #[test]
    fn what_a_device_is_follows_from_its_parts() {
        let one = |classes: &[&str]| {
            let parts: Vec<Part> = classes.iter().map(|class| part(r"USB\X\1", "{c}", class)).collect();
            kind(&parts.iter().collect::<Vec<_>>())
        };
        assert_eq!(one(&["Monitor", "AudioEndpoint", "MEDIA"]), Kind::Display);
        assert_eq!(one(&["Camera", "AudioEndpoint"]), Kind::Camera);
        // A headset whose buttons count as a keyboard.
        assert_eq!(one(&["AudioEndpoint", "HIDClass", "Keyboard"]), Kind::Audio);
        assert_eq!(one(&["Printer", "Image", "DiskDrive"]), Kind::Printer);
        // A scanner, and a webcam whose driver files it with scanners.
        assert_eq!(one(&["Image", "USB"]), Kind::Printer);
        assert_eq!(one(&["Image", "MEDIA", "AudioEndpoint"]), Kind::Camera);
        assert_eq!(one(&["XboxComposite", "AudioEndpoint"]), Kind::Controller);
        assert_eq!(one(&["Mouse", "Keyboard"]), Kind::Input);
        assert_eq!(one(&["DiskDrive", "Volume"]), Kind::Storage);
        assert_eq!(one(&["wpd"]), Kind::Storage);
        assert_eq!(one(&["Net", "USB"]), Kind::Other);
        assert_eq!(one(&[]), Kind::Other);
    }

    #[test]
    fn a_display_is_named_by_its_model() {
        assert_eq!(name(&container("{c}", "Generic Monitor (BenQ GL2780)", "BenQ GL2780"), Kind::Display), "BenQ GL2780");
        // A model that isn't part of the name says less than the name.
        assert_eq!(name(&container("{c}", "LG ULTRAGEAR", "PnP-Monitor (Standard)"), Kind::Display), "LG ULTRAGEAR");
        // Other devices keep the name they were given, which may be the owner's own.
        assert_eq!(name(&container("{c}", "Sam's Kiyo (Razer Kiyo)", "Razer Kiyo"), Kind::Camera), "Sam's Kiyo (Razer Kiyo)");
        assert_eq!(name(&container("{c}", " ", "Elements 2621"), Kind::Storage), "Elements 2621");
        assert_eq!(name(&container("{c}", "", ""), Kind::Other), "Unnamed device");
    }

    #[test]
    fn bluetooth_wins_over_usb_and_a_display_has_no_connection_named() {
        let parts = [part(r"BTHENUM\{0000110b}_VID&0001004c\8&1", "{c}", "MEDIA"), part(r"USB\VID_1\1", "{c}", "USB")];
        assert_eq!(connection(&parts.iter().collect::<Vec<_>>()), Some(Connection::Bluetooth));
        let parts = [part(r"BTHLEDevice\{00001812}_Dev_VID&02046d\9&2", "{c}", "HIDClass")];
        assert_eq!(connection(&parts.iter().collect::<Vec<_>>()), Some(Connection::Bluetooth));
        let parts = [part(r"DISPLAY\BNQ78EC\5&300D53F2&0&UID4352", "{c}", "Monitor")];
        assert_eq!(connection(&parts.iter().collect::<Vec<_>>()), None);
    }

    #[test]
    fn a_problem_is_said_with_its_code_unless_only_an_apps_component_has_it() {
        let broken = |class: &str, problem: u32| Part { problem, ..part(r"USB\VID_1\1", "{c}", class) };
        let parts = [broken("Camera", 0), broken("MEDIA", CM_PROB_FAILED_INSTALL)];
        assert_eq!(problem(&parts.iter().collect::<Vec<_>>()).as_deref(), Some("No driver installed (code 28)"));
        let parts = [broken("Camera", 0), broken("SoftwareComponent", CM_PROB_FAILED_INSTALL)];
        assert_eq!(problem(&parts.iter().collect::<Vec<_>>()), None);
        assert_eq!(problem_text(CM_PROB_DISABLED), "Turned off in Device Manager (code 22)");
        assert_eq!(problem_text(31), "Not working properly (code 31)");
    }

    #[test]
    fn the_clipboard_gets_each_kind_with_its_devices() {
        let device = |name: &str, kind, connection, problem: Option<&str>| Device {
            name: name.to_owned(),
            kind,
            connection,
            problem: problem.map(str::to_owned),
        };
        let devices = [
            device("BenQ GL2780", Kind::Display, None, None),
            device("LG ULTRAGEAR", Kind::Display, None, None),
            device("Razer Kiyo", Kind::Camera, Some(Connection::Usb), Some("No driver installed (code 28)")),
            device("Xbox Wireless Controller", Kind::Controller, Some(Connection::Bluetooth), None),
        ];
        assert_eq!(
            as_text(&devices),
            "Displays\nBenQ GL2780\nLG ULTRAGEAR\n\nCameras\nRazer Kiyo (USB): No driver installed (code 28)\n\n\
             Game controllers\nXbox Wireless Controller (Bluetooth)"
        );
        assert_eq!(as_text(&[]), "");
    }

    #[test]
    fn a_guid_is_written_as_windows_writes_container_ids() {
        let guid = GUID::from_u128(0x09a767d4_b5c9_5032_bef1_ff372c662ad8);
        assert_eq!(guid_text(&guid), "{09a767d4-b5c9-5032-bef1-ff372c662ad8}");
    }

    /// Lists what is plugged into this PC, to compare with Bluetooth & devices in Settings. Run with
    /// `cargo test devices -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_this_pcs_devices() {
        let started = std::time::Instant::now();
        let devices = connected().unwrap();
        let took = started.elapsed();
        let signal_started = std::time::Instant::now();
        let signal = signal();
        println!("listed in {took:?}, signal {signal} in {:?}", signal_started.elapsed());
        for device in devices {
            println!("{:?} | {} | {:?} | {:?}", device.kind, device.name, device.connection, device.problem);
        }
    }
}
