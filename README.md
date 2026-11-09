# Update Check

A small Windows app that reminds you once a week to work through a checklist of update steps: Windows
Update, driver and graphics driver updates, Store apps, winget, Defender, freeing up disk space and backups
by default. Around the checklist it removes the apps that came with the PC, shows what is inside the PC and
resets USB drives that act up. It is a single .exe of about 10 MB, written in Rust, that starts with
Windows if you like and keeps itself up to date from GitHub releases.

## How it works

### Checklist

- Tick steps off as you go, with a click anywhere on a card. **Open** jumps to the matching settings page
  or runs the step's command.
- The checklist resets every week at the reminder day and time chosen in **Settings**, where you can also
  add, remove and reorder steps and switch between light and dark mode. A removed step comes back with
  **Undo** or Ctrl+Z for a few seconds afterwards.
- **What's actually waiting.** The Windows Update, Optional driver updates and Desktop apps steps say how
  much is queued up, for example "2 upgrades waiting", looked up in the background while you read the rest.
  Desktop apps names the packages winget can upgrade, and all three look again when you come back to the
  window after installing something. The Windows Update step also says when Windows last installed updates,
  and driver updates include firmware, which on many brand PCs means the BIOS. Neither tool has a
  machine-readable count: winget prints a table in your own language and Windows Update answers only
  through COM. So both are read defensively and say nothing rather than guess. One Windows Update search
  serves both of its steps and the PC info tab.
- **Free space.** The Free up space step says how much room is left on the drive Windows is on, and stands
  out below 20 GB, about what Windows needs to install a new version of itself.
- **Last done and streaks.** Each step remembers the day it was last ticked and says so when it has been a
  while, and the header counts the weeks you have finished in a row.
- **Graphics drivers.** This step lists the PC's NVIDIA, AMD and Intel graphics with their installed
  drivers. For NVIDIA it looks up the latest driver online, through the undocumented lookup behind NVIDIA's
  driver search, and says whether an update is available; AMD and Intel offer no such lookup. Each card gets
  a button for NVIDIA App, AMD Software: Adrenalin Edition or Intel Graphics Software, or for the driver
  download page if that app isn't installed. Removing the step in **Settings** also stops the online lookup.

### Apps

- The **Apps** tab lists the apps that came with Windows or the PC and that hardly anyone uses, such as
  Solitaire Collection, the MSN apps, Clipchamp or preinstalled games, with the ones worth removing ticked.
  The top of the tab says how many could still go and where.
- **Remove** asks first, then removes the ticked apps for your account only, without administrator rights.
  The Microsoft Store has every one of them if you want one back. Everything else, including what Windows
  needs and everyday apps such as Calculator, Photos or Notepad, is never offered.
- Microsoft Edge (where Windows allows it, in the European Economic Area), OneDrive and trial software can
  only be removed in Windows' Settings, so the tab says step by step how.
- Each removal is written to `removed-apps.log` beside the checklist, one line per app, with the reason in
  plain words where one couldn't be removed.

### PC info

- A second tab lists what the PC is: maker and model, processor, memory, graphics, drives, the Windows
  edition, version and build, the organization that manages the PC (an Active Directory domain or Microsoft
  Entra ID), the BIOS version and how old it is, the boot mode, Secure Boot and the TPM. It is read from
  the registry, the firmware's tables and Windows itself, in milliseconds and without admin rights.
- **Show details** adds what matters when buying parts or asking for help: each memory module with its
  maker, part number, size and speed, the processor's cores, threads and caches, each graphics card's video
  memory, and every drive with its model, size, connection and volumes.
- **Copy to clipboard** puts it all in plain text.
- *Is Windows up to date?* The tab answers the way Windows does: by asking Windows Update whether anything
  is waiting, and whether a restart is needed to finish installing updates.
- *Installed on* is the day Windows was first installed on the PC. Windows' own Settings show the day of
  the last upgrade to a new version instead, which Windows also records as its install date.
- *Is the BIOS up to date?* Nothing on the PC can tell, and every maker publishes BIOS versions
  differently, so the tab shows the BIOS's age and a **Find BIOS updates** button. It opens the support site
  of Dell, HP or Lenovo, which finds the PC's downloads by itself, Windows Update for Surface, or else a web
  search for the model's BIOS.
- **Show connected devices**, beside it, opens a page of everything plugged into the PC or paired with it and
  connected, by kind: displays, keyboards and mice, audio, cameras, game controllers, drives and phones,
  printers and scanners. Each device says whether it is connected by USB or Bluetooth and what Windows finds
  wrong with it, such as a missing driver. The list follows along when something is plugged in or out.
  **Copy** beside a device copies that device, **Copy all** the whole list; the PC info tab's own copy
  leaves the devices out.

### When Windows misbehaves

These tools are at the end of **Settings**.

- **Check system files** runs System File Checker (`sfc /scannow`). **Repair Windows** first repairs
  Windows' own copy of its files with DISM, then runs System File Checker. Both ask for administrator rights
  and run in a console window that stays open.
- **Reset a USB drive** erases a USB stick or memory card and sets it up again as one empty drive, for when
  it shows the wrong size, won't format or still holds an old Windows installer. Erasing the wrong drive
  can't be undone, so:
  - only USB drives and memory cards are offered, never one that Windows or this app runs from;
  - you pick the drive, see what is on it, choose its file system (exFAT, NTFS, or FAT32 up to 32 GB) and
    name, and type `ERASE` to confirm;
  - the script that erases runs as administrator and checks all of this again right before it does.

### Starts with Windows, if you like

On its first start the .exe asks whether to remind you every week. With a yes it registers a sign-in entry
for your user and a daily Task Scheduler task of your own at the reminder hour (no admin rights needed), and
re-registers them if the .exe has moved. Both run `UpdateCheck.exe --check`, which opens the window only
while this week's checklist is unfinished, at most once a day, and not while a full-screen game or a
presentation has the screen. Such a check takes about 20 ms when nothing is due. The **Automatic reminder**
switch in the footer turns this on and off.

### Updates itself

At most once a day the .exe asks GitHub for the latest release of `Zorakidd/Update-check` (`GITHUB_REPO` in
[updater.rs](src/updater.rs)). If its tag is a newer version and an .exe is attached, the app downloads it,
checks its size and the SHA-256 digest GitHub publishes for it (a download without one isn't installed),
swaps it in for the running .exe and restarts with the same arguments. Failures are logged and never stop
the app; a repository without any release yet is no failure. **Settings** shows the running version, and
**Check now** asks straight away instead of waiting for the daily check.

### Keyboard

Tab moves between controls and Space or Enter presses the one in focus. Page Up, Page Down, Home and End
scroll the page, Escape goes back from Settings and the USB page, and Ctrl+Z undoes removing a step.

Steps and progress are stored in `%APPDATA%\UpdateCheck\state.json`, next to `update-check.log` and
`removed-apps.log`. Set `UPDATE_CHECK_DATA` to use a different state file.

## For IT

- **Policy.** Set the DWORD `DisableSelfUpdate` to `1` under `HKLM\SOFTWARE\Policies\UpdateCheck`, by Group
  Policy Preferences, an Intune script or any other way of setting a registry value, to roll out new
  versions yourself. The app then never contacts GitHub, and Settings says that your organization keeps it
  up to date.
- **Rights.** Nothing needs administrator rights except Check system files, Repair Windows and Reset a USB
  drive, which ask through UAC. The reminder is a per-user sign-in entry (`HKCU\...\Run`) and a per-user
  Task Scheduler task, `UpdateCheck (<user name>)`, so several users on one PC each have their own. Apps
  are only ever removed for the signed-in user.
- **Network.** The app itself contacts only GitHub, at most once a day for its own releases, and NVIDIA's
  driver search when the PC has NVIDIA graphics. Requests go through WinHTTP with the PC's proxy settings
  and Windows' certificate store. winget and Windows Update reach their own servers. There is no telemetry.
- **Logs.** `%APPDATA%\UpdateCheck\update-check.log` holds warnings and unexpected errors, and
  `removed-apps.log` what the Apps tab removed. They are kept to about 1 MB and 256 KB, with the previous
  log beside each as `.log.1`.
- **Removal.** Turn off the **Automatic reminder** switch, which deletes the sign-in entry and the task,
  then delete the .exe and `%APPDATA%\UpdateCheck`.

## Develop

```
cargo run
cargo test
cargo clippy --all-targets
```

It builds with stable Rust (edition 2024) and either Windows toolchain; [.cargo/config.toml](.cargo/config.toml)
explains the linker the GNU one needs. The window is drawn with egui on Direct3D 12.

A debug build never updates itself or registers with Windows startup on its own; the footer switch still
works. For testing the window without disturbing the desktop, `UPDATE_CHECK_WINDOW_POS=x,y` opens it at
that position without taking the focus (for example `-20000,-20000`, off screen), and
`UPDATE_CHECK_RELEASES_URL` points the updater elsewhere.

The icon, [icon.ico](assets/icon.ico), is drawn by `uv run --no-project --with pillow python tools/make_icon.py`.
[build.rs](build.rs) embeds it in the .exe with the version information and the application manifest,
without a resource compiler.

## Build and release

1. Bump `version` in `Cargo.toml`.
2. Build the .exe:

   ```
   cargo build --release
   ```

3. Publish a GitHub release tagged `v<version>` (for example `v0.2.0`) with
   `target/release/UpdateCheck.exe` attached. The repository has to be public, since the updater doesn't
   authenticate. Copies of the earlier Python version (0.1) update to it like to any other release: they
   keep their checklist and their reminder.
