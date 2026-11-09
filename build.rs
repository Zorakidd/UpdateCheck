//! Embeds the icon, the version information and the application manifest into the .exe.
//!
//! No resource compiler is needed: the resources are written straight into a COFF object, the format a
//! resource compiler produces, and handed to the linker.

use std::env;
use std::fs;
use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
const LANGUAGE: u32 = 0x0409;

// asInvoker matters beyond the obvious: without a requested execution level, Windows takes any .exe with
// "update" in its name for an installer and asks for administrator rights.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Zorakidd.UpdateCheck" version="{version}" processorArchitecture="amd64"/>
  <description>Update Check</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

struct IconImage {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bits: u16,
    data: Vec<u8>,
}

impl IconImage {
    fn size(&self) -> u32 {
        if self.width == 0 { 256 } else { u32::from(self.width) }
    }
}

struct Resource {
    kind: u16,
    id: u16,
    data: Vec<u8>,
}

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let version = version();
    println!("cargo:rustc-env=UPDATE_CHECK_VERSION={version}");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let ico = fs::read("assets/icon.ico").expect("assets/icon.ico");
    let images = parse_ico(&ico);

    // The title bar and taskbar icon, decoded when the window opens: the largest image up to 128 pixels.
    let window_icon = images
        .iter()
        .filter(|image| image.size() <= 128)
        .max_by_key(|image| image.size())
        .expect("icon.ico has no image up to 128 pixels");
    fs::write(out.join("window-icon.png"), &window_icon.data).expect("write window-icon.png");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let object = coff(&resources(&images, &version));
        let path = out.join("resources.o");
        fs::write(&path, object).expect("write resources.o");
        // An object on the command line is linked in whole, unlike a member of a library.
        println!("cargo:rustc-link-arg-bins={}", path.display());
    }
}

/// Cargo.toml's major and minor version, with the number of commits as the patch version. Each commit counts
/// the version up by itself, and a build of the same commit has the same version on every PC and in the
/// release workflow. Outside a git repository, Cargo.toml's version as it is.
fn version() -> String {
    let package = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let Some(commits) = git(&["rev-list", "--count", "HEAD"]) else { return package };
    // A commit, checkout or reset moves HEAD, which rewrites a branch's ref, or HEAD itself, and adds to
    // HEAD's reflog. Watching a file that doesn't exist would run this script on every build.
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    for file in ["HEAD", "logs/HEAD", "packed-refs"].into_iter().chain(branch.as_deref()) {
        if let Some(path) = git(&["rev-parse", "--git-path", file]).filter(|path| PathBuf::from(path).exists()) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let major = env::var("CARGO_PKG_VERSION_MAJOR").expect("CARGO_PKG_VERSION_MAJOR");
    let minor = env::var("CARGO_PKG_VERSION_MINOR").expect("CARGO_PKG_VERSION_MINOR");
    format!("{major}.{minor}.{commits}")
}

/// What git prints, trimmed, if it runs and succeeds and prints anything.
fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn parse_ico(data: &[u8]) -> Vec<IconImage> {
    assert!(data.len() >= 6 && u16_at(data, 2) == 1, "not an .ico file");
    (0..usize::from(u16_at(data, 4)))
        .map(|index| {
            let entry = 6 + 16 * index;
            let length = u32_at(data, entry + 8) as usize;
            let offset = u32_at(data, entry + 12) as usize;
            IconImage {
                width: data[entry],
                height: data[entry + 1],
                colors: data[entry + 2],
                planes: u16_at(data, entry + 4),
                bits: u16_at(data, entry + 6),
                data: data[offset..offset + length].to_vec(),
            }
        })
        .collect()
}

fn resources(images: &[IconImage], version: &str) -> Vec<Resource> {
    let mut resources = Vec::new();
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((images.len() as u16).to_le_bytes());
    for (index, image) in images.iter().enumerate() {
        let id = index as u16 + 1;
        group.extend([image.width, image.height, image.colors, 0]);
        group.extend(image.planes.to_le_bytes());
        group.extend(image.bits.to_le_bytes());
        group.extend((image.data.len() as u32).to_le_bytes());
        group.extend(id.to_le_bytes());
        resources.push(Resource { kind: RT_ICON, id, data: image.data.clone() });
    }
    resources.push(Resource { kind: RT_GROUP_ICON, id: 1, data: group });
    resources.push(Resource { kind: RT_VERSION, id: 1, data: version_info(version) });
    let manifest = MANIFEST.replace("{version}", &four_part(version).map(|part| part.to_string()).join("."));
    resources.push(Resource { kind: RT_MANIFEST, id: 1, data: manifest.into_bytes() });
    resources.sort_by_key(|resource| (resource.kind, resource.id));
    resources
}

fn four_part(version: &str) -> [u16; 4] {
    let mut parts = [0u16; 4];
    for (slot, part) in parts.iter_mut().zip(version.split(['.', '-', '+'])) {
        *slot = part.parse().unwrap_or(0);
    }
    parts
}

enum Value {
    None,
    Text(String),
    Binary(Vec<u8>),
}

struct Node {
    key: &'static str,
    value: Value,
    children: Vec<Node>,
}

fn leaf(key: &'static str, value: Value) -> Node {
    Node { key, value, children: Vec::new() }
}

fn pad4(buffer: &mut Vec<u8>) {
    while !buffer.len().is_multiple_of(4) {
        buffer.push(0);
    }
}

fn utf16z(buffer: &mut Vec<u8>, text: &str) {
    for unit in text.encode_utf16().chain([0]) {
        buffer.extend(unit.to_le_bytes());
    }
}

fn write_node(node: &Node, buffer: &mut Vec<u8>) {
    pad4(buffer);
    let start = buffer.len();
    let (value_length, kind) = match &node.value {
        Value::None => (0, 1u16),
        Value::Text(text) => (text.encode_utf16().count() + 1, 1),
        Value::Binary(bytes) => (bytes.len(), 0),
    };
    buffer.extend([0, 0]);
    buffer.extend((value_length as u16).to_le_bytes());
    buffer.extend(kind.to_le_bytes());
    utf16z(buffer, node.key);
    pad4(buffer);
    match &node.value {
        Value::None => {}
        Value::Text(text) => utf16z(buffer, text),
        Value::Binary(bytes) => buffer.extend(bytes),
    }
    for child in &node.children {
        write_node(child, buffer);
    }
    let length = (buffer.len() - start) as u16;
    buffer[start..start + 2].copy_from_slice(&length.to_le_bytes());
}

fn version_info(version: &str) -> Vec<u8> {
    let [a, b, c, d] = four_part(version).map(u32::from);
    let (most, least) = ((a << 16) | b, (c << 16) | d);
    let mut fixed = Vec::new();
    // VS_FIXEDFILEINFO: signature, structure version, file and product version, flags mask and flags,
    // VOS_NT_WINDOWS32, VFT_APP, no subtype and no date.
    for value in [0xFEEF_04BD, 0x0001_0000, most, least, most, least, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        fixed.extend(u32::to_le_bytes(value));
    }
    let strings = [
        ("CompanyName", "Zorakidd"),
        ("FileDescription", "Update Check"),
        ("FileVersion", version),
        ("InternalName", "UpdateCheck"),
        ("LegalCopyright", "Copyright (c) 2026 Zorakidd"),
        ("OriginalFilename", "UpdateCheck.exe"),
        ("ProductName", "Update Check"),
        ("ProductVersion", version),
    ];
    let mut translation = Vec::new();
    translation.extend(0x0409u16.to_le_bytes());
    translation.extend(1200u16.to_le_bytes());
    let root = Node {
        key: "VS_VERSION_INFO",
        value: Value::Binary(fixed),
        children: vec![
            Node {
                key: "StringFileInfo",
                value: Value::None,
                children: vec![Node {
                    key: "040904B0",
                    value: Value::None,
                    children: strings
                        .iter()
                        .map(|(key, value)| leaf(key, Value::Text((*value).to_owned())))
                        .collect(),
                }],
            },
            Node {
                key: "VarFileInfo",
                value: Value::None,
                children: vec![leaf("Translation", Value::Binary(translation))],
            },
        ],
    };
    let mut buffer = Vec::new();
    write_node(&root, &mut buffer);
    buffer
}

fn align8(offset: usize) -> usize {
    (offset + 7) & !7
}

fn directory(buffer: &mut Vec<u8>, id_entries: usize) {
    buffer.extend([0u8; 12]);
    buffer.extend(0u16.to_le_bytes());
    buffer.extend((id_entries as u16).to_le_bytes());
}

fn entry(buffer: &mut Vec<u8>, id: u32, offset: u32) {
    buffer.extend(id.to_le_bytes());
    buffer.extend(offset.to_le_bytes());
}

/// A COFF object with a single .rsrc section holding a resource tree three levels deep: type, id, language.
fn coff(resources: &[Resource]) -> Vec<u8> {
    const SUBDIRECTORY: u32 = 0x8000_0000;
    let mut kinds: Vec<u16> = resources.iter().map(|resource| resource.kind).collect();
    kinds.dedup();

    let mut offset = 16 + 8 * kinds.len();
    let mut kind_offsets = Vec::new();
    for kind in &kinds {
        kind_offsets.push(offset);
        offset += 16 + 8 * resources.iter().filter(|resource| resource.kind == *kind).count();
    }
    let mut language_offsets = Vec::new();
    for _ in resources {
        language_offsets.push(offset);
        offset += 16 + 8;
    }
    let mut entry_offsets = Vec::new();
    for _ in resources {
        entry_offsets.push(offset);
        offset += 16;
    }
    let mut data_offsets = Vec::new();
    for resource in resources {
        offset = align8(offset);
        data_offsets.push(offset);
        offset += resource.data.len();
    }

    let mut section = Vec::new();
    directory(&mut section, kinds.len());
    for (kind, kind_offset) in kinds.iter().zip(&kind_offsets) {
        entry(&mut section, u32::from(*kind), SUBDIRECTORY | *kind_offset as u32);
    }
    for kind in &kinds {
        let members: Vec<usize> = (0..resources.len()).filter(|&i| resources[i].kind == *kind).collect();
        directory(&mut section, members.len());
        for index in members {
            entry(&mut section, u32::from(resources[index].id), SUBDIRECTORY | language_offsets[index] as u32);
        }
    }
    for entry_offset in &entry_offsets {
        directory(&mut section, 1);
        entry(&mut section, LANGUAGE, *entry_offset as u32);
    }
    for (resource, data_offset) in resources.iter().zip(&data_offsets) {
        // OffsetToData is an address in the image, filled in by the linker through the relocation below.
        section.extend((*data_offset as u32).to_le_bytes());
        section.extend((resource.data.len() as u32).to_le_bytes());
        section.extend(0u32.to_le_bytes());
        section.extend(0u32.to_le_bytes());
    }
    for (resource, data_offset) in resources.iter().zip(&data_offsets) {
        section.resize(*data_offset, 0);
        section.extend(&resource.data);
    }
    section.resize(align8(section.len()), 0);

    const HEADERS: usize = 20 + 40;
    let relocations_at = HEADERS + section.len();
    let symbols_at = relocations_at + 10 * resources.len();
    let mut object = Vec::new();
    // IMAGE_FILE_HEADER: x86-64, one section, the symbol table, two symbol records, no optional header.
    object.extend(0x8664u16.to_le_bytes());
    object.extend(1u16.to_le_bytes());
    object.extend(0u32.to_le_bytes());
    object.extend((symbols_at as u32).to_le_bytes());
    object.extend(2u32.to_le_bytes());
    object.extend(0u16.to_le_bytes());
    object.extend(0u16.to_le_bytes());
    // IMAGE_SECTION_HEADER: initialized, readable data, aligned to 8 bytes.
    object.extend(*b".rsrc\0\0\0");
    object.extend(0u32.to_le_bytes());
    object.extend(0u32.to_le_bytes());
    object.extend((section.len() as u32).to_le_bytes());
    object.extend((HEADERS as u32).to_le_bytes());
    object.extend((relocations_at as u32).to_le_bytes());
    object.extend(0u32.to_le_bytes());
    object.extend((resources.len() as u16).to_le_bytes());
    object.extend(0u16.to_le_bytes());
    object.extend(0x4040_0040u32.to_le_bytes());
    object.extend(&section);
    for entry_offset in &entry_offsets {
        // IMAGE_REL_AMD64_ADDR32NB against the section's symbol: an address relative to the image base.
        object.extend((*entry_offset as u32).to_le_bytes());
        object.extend(0u32.to_le_bytes());
        object.extend(3u16.to_le_bytes());
    }
    // The section symbol, static, with its auxiliary section definition.
    object.extend(*b".rsrc\0\0\0");
    object.extend(0u32.to_le_bytes());
    object.extend(1i16.to_le_bytes());
    object.extend(0u16.to_le_bytes());
    object.push(3);
    object.push(1);
    object.extend((section.len() as u32).to_le_bytes());
    object.extend((resources.len() as u16).to_le_bytes());
    object.extend(0u16.to_le_bytes());
    object.extend(0u32.to_le_bytes());
    object.extend(0u16.to_le_bytes());
    object.extend([0u8; 4]);
    // An empty string table.
    object.extend(4u32.to_le_bytes());
    object
}
