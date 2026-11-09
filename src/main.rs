//! Update Check: a weekly checklist that keeps a Windows PC up to date, with a reminder, the PC's details and
//! a cleanup of the apps that came with it.

// A windowed program, so starting it opens no console; the tests keep theirs for their output.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod applog;
mod apps;
mod devices;
mod graphics;
mod hardware;
mod http;
mod pcinfo;
mod pending;
mod startup;
mod state;
mod system;
mod ui;
mod updater;
mod usb;

use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage: UpdateCheck [-h] [--check]

Weekly update checklist with reminders.

options:
  -h, --help  show this help message and exit
  --check     only open the window if this week's checklist is due (used by the
              automatic reminder)
";

fn main() -> ExitCode {
    applog::install_panic_hook();
    let mut check_only = false;
    let mut unknown = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("-h" | "--help") => {
                print(USAGE, false);
                return ExitCode::SUCCESS;
            }
            Some("--check") => check_only = true,
            _ => unknown.push(arg.to_string_lossy().into_owned()),
        }
    }
    if !unknown.is_empty() {
        print(&format!("{USAGE}\nunrecognized arguments: {}\n", unknown.join(" ")), true);
        return ExitCode::from(2);
    }

    let code = startup::run(check_only, app::open_window);
    if app::restart_requested() {
        // Started again as if by hand, so the window opens whatever is due.
        if let Err(error) = std::process::Command::new(system::exe_path()).spawn() {
            applog::error("startup", format!("Couldn't start the new version: {error}"));
        }
    }
    code
}

/// Write to where the app was started from. Output sent to a file or a pipe goes there; otherwise, as a
/// windowed program has no console of its own, it borrows the one of the command prompt that started it.
fn print(text: &str, error: bool) {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    let handle = unsafe { GetStdHandle(if error { STD_ERROR_HANDLE } else { STD_OUTPUT_HANDLE }) };
    if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
        let _ = if error { std::io::stderr().write_all(text.as_bytes()) } else { std::io::stdout().write_all(text.as_bytes()) };
        return;
    }
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0
        && let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$")
    {
        // A line break first, since the prompt came back as soon as the program started.
        let _ = console.write_all(format!("\n{text}").as_bytes());
    }
}
