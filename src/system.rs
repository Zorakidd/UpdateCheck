//! Windows itself: the registry, the shell, processes, windows, and starting with Windows.

use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HWND, LPARAM, RECT, WAIT_OBJECT_0};
use windows_sys::Win32::Globalization::{CP_OEMCP, MultiByteToWideChar};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_DWORD, REG_SZ,
    RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_QWORD, RRF_RT_REG_SZ,
    RRF_SUBKEY_WOW6464KEY, RegCloseKey, RegDeleteValueW, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Shell::{
    QUERY_USER_NOTIFICATION_STATE, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW, SHQueryUserNotificationState, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, MessageBoxW, SPI_GETWORKAREA, SW_RESTORE,
    SW_HIDE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow, SystemParametersInfoW,
};

pub const APP_NAME: &str = "UpdateCheck";
pub const TITLE: &str = "Update Check";
pub const USER_AGENT: &str = "Update-Check";
/// Cargo.toml's major and minor version, with the number of commits as the patch version: see build.rs.
pub const VERSION: &str = env!("UPDATE_CHECK_VERSION");

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const POLICY_KEY: &str = r"SOFTWARE\Policies\UpdateCheck";
const INSTANCE_MUTEX: &str = r"Local\UpdateCheck.SingleInstance";

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
/// The user cancelled a prompt, such as the one asking for administrator rights.
pub const ERROR_CANCELLED: u32 = 1223;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_MORE_DATA: u32 = 234;

/// Text as Windows wants it: UTF-16 with a terminating zero.
pub fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain([0]).collect()
}

/// UTF-16 text up to its terminating zero, if any.
pub fn from_wide(text: &[u16]) -> String {
    let end = text.iter().position(|unit| *unit == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

/// Windows' own description of an error code, such as "Access is denied."
pub fn error_text(code: u32) -> String {
    let mut buffer = [0u16; 512];
    let length = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            code,
            0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            std::ptr::null(),
        )
    };
    let text = String::from_utf16_lossy(&buffer[..length as usize]).trim().to_owned();
    if text.is_empty() { format!("Error {code}") } else { text }
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// A function of one of Windows' DLLs that is looked up when it is needed, for one that older versions of
/// Windows 10 lack: linking to it would keep the app from starting on those at all. The DLL stays loaded.
///
/// # Safety
/// `F` must be an `unsafe extern "system" fn` type with the function's exact signature.
pub unsafe fn system_function<F: Copy>(dll: &str, name: &std::ffi::CStr) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<usize>()) };
    let module = unsafe { LoadLibraryExW(wide(dll).as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return None;
    }
    let address = unsafe { GetProcAddress(module, name.as_ptr().cast()) }?;
    Some(unsafe { std::mem::transmute_copy(&address) })
}

/// UTF-16 text that a Windows function handed out as a pointer to its first character.
///
/// # Safety
/// `text` must be null or point to text that ends in a zero.
pub unsafe fn from_wide_pointer(text: *const u16) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) })
}

/// Where Windows lives, normally C:\Windows.
pub fn windows_dir() -> PathBuf {
    std::env::var_os("SystemRoot").filter(|root| !root.is_empty()).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

/// A program of Windows', by its full path, so a file of the same name elsewhere on PATH is never run.
pub fn system32(name: &str) -> PathBuf {
    windows_dir().join("System32").join(name)
}

/// The user's Downloads folder, wherever it was moved to.
pub fn downloads_dir() -> Option<PathBuf> {
    use windows_sys::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath};
    let mut path: *mut u16 = std::ptr::null_mut();
    let result = unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut path) };
    let found = (result == 0 && !path.is_null()).then(|| PathBuf::from(unsafe { from_wide_pointer(path) }));
    unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(path.cast()) };
    found.or_else(|| std::env::var_os("USERPROFILE").map(|profile| PathBuf::from(profile).join("Downloads")))
}

pub fn powershell() -> PathBuf {
    system32(r"WindowsPowerShell\v1.0\powershell.exe")
}

/// winget, which the App Installer puts among the user's app execution aliases.
pub fn winget() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let alias = PathBuf::from(local).join(r"Microsoft\WindowsApps\winget.exe");
        if alias.exists() {
            return alias;
        }
    }
    PathBuf::from("winget.exe")
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from(format!("{APP_NAME}.exe")))
}

/// Whether this is the release build that users run, which registers with Windows and updates itself.
/// Development builds leave both alone.
pub const fn is_release() -> bool {
    !cfg!(debug_assertions)
}

// The registry.

#[derive(Clone, Copy, Debug)]
pub enum Hive {
    LocalMachine,
    CurrentUser,
}

impl Hive {
    fn handle(self) -> HKEY {
        match self {
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            Hive::CurrentUser => HKEY_CURRENT_USER,
        }
    }
}

/// A value's bytes, read from the 64-bit view of the registry, if it has one of the types in `flags`.
fn reg_value(hive: Hive, key: &str, value: &str, flags: u32) -> Option<Vec<u8>> {
    let key = wide(key);
    let value = wide(value);
    let mut size = 0u32;
    let flags = flags | RRF_SUBKEY_WOW6464KEY;
    let status = unsafe {
        RegGetValueW(hive.handle(), key.as_ptr(), value.as_ptr(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
    };
    if status != 0 {
        return None;
    }
    // The value can grow between the two calls.
    for _ in 0..4 {
        let mut data = vec![0u8; size as usize + 8];
        size = data.len() as u32;
        let status = unsafe {
            RegGetValueW(
                hive.handle(),
                key.as_ptr(),
                value.as_ptr(),
                flags,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match status {
            0 => {
                data.truncate(size as usize);
                return Some(data);
            }
            ERROR_MORE_DATA => continue,
            _ => return None,
        }
    }
    None
}

fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect()
}

/// A text value, with environment variables in it expanded. Empty text counts as none.
pub fn reg_string(hive: Hive, key: &str, value: &str) -> Option<String> {
    let text = from_wide(&utf16(&reg_value(hive, key, value, RRF_RT_REG_SZ)?)).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

pub fn reg_dword(hive: Hive, key: &str, value: &str) -> Option<u32> {
    let bytes = reg_value(hive, key, value, RRF_RT_REG_DWORD)?;
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

pub fn reg_qword(hive: Hive, key: &str, value: &str) -> Option<u64> {
    let bytes = reg_value(hive, key, value, RRF_RT_REG_QWORD)?;
    Some(u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?))
}

pub fn reg_binary(hive: Hive, key: &str, value: &str) -> Option<Vec<u8>> {
    reg_value(hive, key, value, RRF_RT_REG_BINARY)
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_key(hive: Hive, key: &str, access: u32) -> Result<Key, u32> {
    let mut handle: HKEY = std::ptr::null_mut();
    let status = unsafe { RegOpenKeyExW(hive.handle(), wide(key).as_ptr(), 0, access | KEY_WOW64_64KEY, &mut handle) };
    if status == 0 { Ok(Key(handle)) } else { Err(status) }
}

pub fn reg_key_exists(hive: Hive, key: &str) -> bool {
    open_key(hive, key, KEY_READ).is_ok()
}

/// The names of a key's subkeys.
pub fn reg_subkeys(hive: Hive, key: &str) -> Vec<String> {
    let Ok(key) = open_key(hive, key, KEY_READ) else { return Vec::new() };
    let mut names = Vec::new();
    for index in 0.. {
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut length,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            break;
        }
        names.push(String::from_utf16_lossy(&name[..length as usize]));
    }
    names
}

fn set_reg_string(hive: Hive, key: &str, value: &str, data: &str) -> io::Result<()> {
    let key = open_key(hive, key, KEY_SET_VALUE).map_err(os_error)?;
    let data = wide(data);
    let status = unsafe {
        RegSetValueExW(key.0, wide(value).as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
    };
    if status == 0 { Ok(()) } else { Err(os_error(status)) }
}

/// Sets a DWORD in a key that already exists.
pub fn set_reg_dword(hive: Hive, key: &str, value: &str, data: u32) -> io::Result<()> {
    let key = open_key(hive, key, KEY_SET_VALUE).map_err(os_error)?;
    let data = data.to_le_bytes();
    let status = unsafe { RegSetValueExW(key.0, wide(value).as_ptr(), 0, REG_DWORD, data.as_ptr(), data.len() as u32) };
    if status == 0 { Ok(()) } else { Err(os_error(status)) }
}

fn delete_reg_value(hive: Hive, key: &str, value: &str) -> io::Result<()> {
    let key = match open_key(hive, key, KEY_SET_VALUE) {
        Ok(key) => key,
        Err(ERROR_FILE_NOT_FOUND) => return Ok(()),
        Err(status) => return Err(os_error(status)),
    };
    match unsafe { RegDeleteValueW(key.0, wide(value).as_ptr()) } {
        0 | ERROR_FILE_NOT_FOUND => Ok(()),
        status => Err(os_error(status)),
    }
}

fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

/// A policy the organization set for the app, if it set it as a DWORD.
pub fn policy(name: &str) -> Option<u32> {
    reg_dword(Hive::LocalMachine, POLICY_KEY, name)
}

/// Whether the app may update itself: always, unless the organization rolls out its versions itself.
pub fn self_update_allowed() -> bool {
    policy("DisableSelfUpdate") != Some(1)
}

// Disks.

/// The drive Windows is on, such as "C:".
pub fn system_drive() -> String {
    std::env::var("SystemDrive").ok().filter(|drive| !drive.is_empty()).unwrap_or_else(|| "C:".to_owned())
}

/// Free and total bytes of the volume `path` is on.
pub fn disk_space(path: &str) -> Option<(u64, u64)> {
    let mut path = path.to_owned();
    if !path.ends_with('\\') {
        path.push('\\');
    }
    let (mut free, mut total) = (0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(wide(&path).as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((free, total))
}

/// The drive Windows is on with its free and total bytes.
pub fn system_drive_space() -> Option<(String, u64, u64)> {
    let drive = system_drive();
    let (free, total) = disk_space(&drive)?;
    Some((drive, free, total))
}

/// A size as Explorer shows it, in GB or TB counted in powers of 1024: "954 GB", "15.9 GB", "1.82 TB".
pub fn size(bytes: u64) -> String {
    let mut value = bytes as f64 / 1024f64.powi(3);
    let mut unit = "GB";
    if value >= 1000.0 {
        value /= 1024.0;
        unit = "TB";
    }
    if value < 1.0 && unit == "GB" && bytes > 0 {
        let megabytes = (bytes as f64 / 1024f64.powi(2)).round();
        return format!("{megabytes} MB");
    }
    let decimals = if value < 10.0 {
        2
    } else if value < 100.0 {
        1
    } else {
        0
    };
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    format!("{text} {unit}")
}

// Running programs.

pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Run a program without a console window, wait for it at most `timeout`, and collect what it printed.
/// A program that runs too long is ended and gives a `TimedOut` error.
pub fn run_hidden(program: &Path, args: &[&str], timeout: Duration) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let reader = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = reader(child.stdout.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>));
    let stderr = reader(child.stderr.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            // The readers end once whatever the program started lets go of its output too.
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the program didn't finish in time"));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    Ok(Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Text a console program printed in the console's code page, as schtasks does.
pub fn decode_oem(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let length = unsafe {
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0)
    };
    if length <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut text = vec![0u16; length as usize];
    unsafe { MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), bytes.len() as i32, text.as_mut_ptr(), length) };
    String::from_utf16_lossy(&text)
}

/// A command line that gives a program exactly these arguments, quoted the way the C runtime reads them.
pub fn command_line<S: AsRef<str>>(args: &[S]) -> String {
    let mut line = String::new();
    for (index, arg) in args.iter().enumerate() {
        let arg = arg.as_ref();
        if index > 0 {
            line.push(' ');
        }
        let quote = arg.is_empty() || arg.contains([' ', '\t']);
        if quote {
            line.push('"');
        }
        let mut backslashes = 0;
        for char in arg.chars() {
            match char {
                '\\' => backslashes += 1,
                '"' => {
                    line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    line.push('"');
                    backslashes = 0;
                }
                _ => {
                    line.extend(std::iter::repeat_n('\\', backslashes));
                    line.push(char);
                    backslashes = 0;
                }
            }
        }
        line.extend(std::iter::repeat_n('\\', if quote { backslashes * 2 } else { backslashes }));
        if quote {
            line.push('"');
        }
    }
    line
}

// The shell.

/// Open a file, folder, web address or settings page the way Explorer would, or run a program with `verb`
/// "runas" as administrator. Returns the started process when `wait` asks for it.
pub fn shell_execute(verb: &str, file: &str, parameters: Option<&str>, wait: bool) -> Result<Option<HANDLE>, u32> {
    let verb = wide(verb);
    let file = wide(file);
    let parameters = parameters.map(wide);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | if wait { SEE_MASK_NOCLOSEPROCESS } else { 0 };
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ref().map_or(std::ptr::null(), |parameters| parameters.as_ptr());
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(last_error());
    }
    Ok((!info.hProcess.is_null()).then_some(info.hProcess))
}

pub fn close_handle(handle: HANDLE) {
    unsafe { CloseHandle(handle) };
}

/// Whether `action` is a URI such as https://… or ms-settings:… rather than a command.
pub fn is_uri(action: &str) -> bool {
    let mut chars = action.chars();
    if !chars.next().is_some_and(|first| first.is_ascii_alphabetic()) {
        return false;
    }
    // A scheme of one letter is a drive, as in C:\Tools.
    for (index, char) in chars.enumerate() {
        if char == ':' {
            return index >= 1;
        }
        if !(char.is_ascii_alphanumeric() || matches!(char, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

/// Do what a step's Open button promises: open a web address, settings page, file or folder, or else run the
/// action as a command in a console window that stays open, so its output can be read. Returns that console,
/// so the window can tell when it has been closed.
pub fn open_action(action: &str) -> Result<Option<Child>, String> {
    let action = action.trim();
    if is_uri(action) || Path::new(action).exists() {
        return shell_execute("open", action, None, false).map(|_| None).map_err(error_text);
    }
    Command::new(system32("cmd.exe"))
        .raw_arg(format!("/k {action}"))
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(Some)
        .map_err(|error| error.to_string())
}

pub fn open_url(url: &str) -> Result<(), String> {
    shell_execute("open", url, None, false).map(|_| ()).map_err(error_text)
}

const REPAIR_SCRIPT: &str = "title Repair Windows & echo Step 1 of 2: Repairing the Windows image with DISM... & \
    DISM /Online /Cleanup-Image /RestoreHealth & echo. & \
    echo Step 2 of 2: Checking system files with System File Checker... & sfc /scannow & echo. & \
    echo Finished. If files were repaired, restart the PC.";

const CHECK_FILES_SCRIPT: &str = "title Check system files & \
    echo Checking system files with System File Checker... & sfc /scannow & echo. & \
    echo Finished. If files were repaired, restart the PC.";

/// Run DISM and System File Checker as administrator, in a console window that stays open. Returns false if
/// the user said no to administrator rights.
pub fn repair_windows() -> Result<bool, String> {
    run_as_admin_console(REPAIR_SCRIPT)
}

/// Run only System File Checker, the quicker half of the repair, as administrator.
pub fn check_system_files() -> Result<bool, String> {
    run_as_admin_console(CHECK_FILES_SCRIPT)
}

fn run_as_admin_console(script: &str) -> Result<bool, String> {
    match shell_execute("runas", &system32("cmd.exe").to_string_lossy(), Some(&format!("/k {script}")), false) {
        Ok(_) => Ok(true),
        Err(ERROR_CANCELLED) => Ok(false),
        Err(code) => Err(error_text(code)),
    }
}

/// Run a program as administrator without a window, and wait up to `timeout` for it to finish. Returns its
/// exit code, or None if it was still running; `Err(ERROR_CANCELLED)` if the user said no.
pub fn run_elevated(program: &Path, parameters: &str, timeout: Duration) -> Result<Option<u32>, u32> {
    let verb = wide("runas");
    let file = wide(program);
    let parameters = wide(parameters);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(last_error());
    }
    if info.hProcess.is_null() {
        return Ok(None);
    }
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    let waited = unsafe { WaitForSingleObject(info.hProcess, milliseconds) };
    let mut code = 0u32;
    let finished = waited == WAIT_OBJECT_0 && unsafe { GetExitCodeProcess(info.hProcess, &mut code) } != 0;
    close_handle(info.hProcess);
    Ok(finished.then_some(code))
}

// Starting with Windows.

/// The command the sign-in entry and the task run.
pub fn reminder_command() -> String {
    command_line(&[exe_path().to_string_lossy().as_ref(), "--check"])
}

/// The user's own task: Task Scheduler's names are machine-wide, and every user gets one.
pub fn task_name() -> String {
    task_name_for(&std::env::var("USERNAME").unwrap_or_default())
}

pub fn task_name_for(user: &str) -> String {
    let user: String = user.chars().map(|char| if r#"\/:*?"<>|"#.contains(char) { '_' } else { char }).collect();
    let user = user.trim();
    format!("{APP_NAME} ({})", if user.is_empty() { "user" } else { user })
}

/// The command the sign-in entry runs, if there is one.
pub fn autostart_command() -> Option<String> {
    reg_string(Hive::CurrentUser, RUN_KEY, APP_NAME)
}

/// Whether the app starts with Windows, which is what the switch in the window shows.
pub fn autostart_enabled() -> bool {
    autostart_command().is_some()
}

/// Make starting with Windows match `enabled`, and point it at this .exe if it has moved.
pub fn sync_autostart(enabled: bool, hour: u32) -> Result<(), String> {
    let registered = autostart_command();
    if !enabled {
        return if registered.is_some() { disable_autostart() } else { Ok(()) };
    }
    if registered.as_deref() != Some(reminder_command().as_str()) {
        enable_autostart(hour)?;
    }
    Ok(())
}

/// Start with Windows, and remind every day at `hour` in case the PC stays on: a sign-in entry and a task
/// of the user's own, neither of which needs administrator rights.
pub fn enable_autostart(hour: u32) -> Result<(), String> {
    let command = reminder_command();
    let start = format!("{:02}:00", hour % 24);
    schtasks(&["/Create", "/F", "/SC", "DAILY", "/ST", &start, "/TN", &task_name(), "/TR", &command], true)?;
    set_reg_string(Hive::CurrentUser, RUN_KEY, APP_NAME, &command).map_err(|error| error.to_string())
}

pub fn disable_autostart() -> Result<(), String> {
    delete_reg_value(Hive::CurrentUser, RUN_KEY, APP_NAME).map_err(|error| error.to_string())?;
    let _ = schtasks(&["/Delete", "/F", "/TN", &task_name()], false);
    Ok(())
}

fn schtasks(args: &[&str], check: bool) -> Result<(), String> {
    let output = match run_hidden(&system32("schtasks.exe"), args, Duration::from_secs(30)) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::TimedOut => return Err("Task Scheduler didn't answer".to_owned()),
        Err(error) => return Err(error.to_string()),
    };
    if !check || output.code == Some(0) {
        return Ok(());
    }
    let message = decode_oem(&output.stderr).trim().to_owned();
    Err(if message.is_empty() {
        format!("schtasks exited with code {}", output.code.unwrap_or(-1))
    } else {
        message
    })
}

// Windows and instances.

/// Holds the app's single-instance lock until dropped.
pub struct InstanceLock(HANDLE);

// The mutex handle is only closed, from whichever thread drops the lock.
unsafe impl Send for InstanceLock {}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        close_handle(self.0);
    }
}

/// The lock that keeps a second copy of the app from opening, or None if another copy holds it.
pub fn claim_single_instance() -> Option<InstanceLock> {
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide(INSTANCE_MUTEX).as_ptr()) };
    if handle.is_null() {
        // Without a lock the app still works; two windows are the worst that can happen.
        return Some(InstanceLock(handle));
    }
    if last_error() == ERROR_ALREADY_EXISTS {
        close_handle(handle);
        return None;
    }
    Some(InstanceLock(handle))
}

/// The full path of the program a process runs.
fn process_image(process_id: u32) -> Option<PathBuf> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    let mut path = [0u16; 1024];
    let mut length = path.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length) };
    close_handle(process);
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&path[..length as usize])))
}

/// Bring the window of the copy of this app that is already open to the front.
pub fn focus_window(title: &str) -> bool {
    struct Search {
        title: String,
        exe: PathBuf,
        found: HWND,
    }
    unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        let mut text = [0u16; 256];
        let length = unsafe { GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32) };
        if length <= 0 || String::from_utf16_lossy(&text[..length as usize]) != search.title {
            return 1;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        let same_program = process_image(process_id).is_some_and(|image| {
            image.to_string_lossy().eq_ignore_ascii_case(&search.exe.to_string_lossy())
        });
        if same_program {
            search.found = window;
            return 0;
        }
        1
    }
    let mut search = Search { title: title.to_owned(), exe: exe_path(), found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    if search.found.is_null() {
        return false;
    }
    unsafe {
        if IsIconic(search.found) != 0 {
            ShowWindow(search.found, SW_RESTORE);
        }
        SetForegroundWindow(search.found);
    }
    true
}

/// Whether a reminder would get in the way: a full-screen game, a presentation, or Windows' own do not
/// disturb for full-screen apps.
pub fn user_is_busy() -> bool {
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    let result = unsafe { SHQueryUserNotificationState(&mut state) };
    // QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN and QUNS_PRESENTATION_MODE.
    result >= 0 && matches!(state, 2..=4)
}

pub fn message_box(text: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(TITLE).as_ptr(), flags) }
}

/// The window's height in points: as designed, or less on a small screen.
pub fn window_height(designed: f32) -> f32 {
    let mut area = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut area as *mut RECT).cast(), 0) };
    let dpi = unsafe { GetDpiForSystem() }.max(96) as f32;
    if ok == 0 || area.bottom <= area.top {
        return designed;
    }
    let available = (area.bottom - area.top) as f32 / (dpi / 96.0) - 40.0;
    designed.min(available.max(400.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_explorer() {
        let gb = 1024u64.pow(3);
        assert_eq!(size(32 * gb), "32 GB");
        assert_eq!(size((15.9 * gb as f64) as u64), "15.9 GB");
        assert_eq!(size(954 * gb), "954 GB");
        assert_eq!(size(1862 * gb), "1.82 TB");
        assert_eq!(size(512 * 1024 * 1024), "512 MB");
    }

    #[test]
    fn task_name_is_the_users_own() {
        assert_eq!(task_name_for("Zoë Müller"), "UpdateCheck (Zoë Müller)");
        assert_eq!(task_name_for(r"DOMAIN\user:1"), "UpdateCheck (DOMAIN_user_1)");
        assert_eq!(task_name_for(""), "UpdateCheck (user)");
    }

    #[test]
    fn command_lines_quote_like_the_c_runtime() {
        assert_eq!(command_line(&[r"C:\Program Files\Update Check\UpdateCheck.exe", "--check"]),
            r#""C:\Program Files\Update Check\UpdateCheck.exe" --check"#);
        assert_eq!(command_line(&[r"C:\Tools\UpdateCheck.exe", "--check"]), r"C:\Tools\UpdateCheck.exe --check");
        assert_eq!(command_line(&["a b\\", "", r#"say "hi""#]), r#""a b\\" "" "say \"hi\"""#);
        assert_eq!(command_line(&[r"back\\slash"]), r"back\\slash");
    }

    #[test]
    fn uris_are_told_from_commands() {
        assert!(is_uri("ms-settings:windowsupdate"));
        assert!(is_uri("https://example.com"));
        assert!(is_uri("ms-windows-store://downloadsandupdates"));
        assert!(!is_uri("winget upgrade --all"));
        assert!(!is_uri(r"C:\Windows"));
        assert!(!is_uri(""));
    }

    #[test]
    fn wide_text_round_trips() {
        assert_eq!(from_wide(&wide("Grüße")), "Grüße");
        assert_eq!(wide("C:\\").last(), Some(&0));
    }
}
