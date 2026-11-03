# Update Check

A small Windows app that reminds you once a week to work through a checklist of update steps:
Windows Update, driver and graphics driver updates, Store apps, winget, Defender, freeing up disk space and
backups by default. It ships as a
single .exe that starts with Windows and keeps itself up to date from GitHub releases.

## How it works

- Tick steps off as you go. **Open** jumps to the matching settings page or runs the step's command.
- The checklist resets every week at the reminder day and time chosen in **Settings**, where you can
  also add, remove and reorder steps and switch between light and dark mode.
- **What's actually waiting.** The Windows Update, Optional driver updates and Desktop apps steps say how
  much is queued up, for example "4 upgrades waiting", looked up in the background while you read the
  rest. The Windows Update step also says when Windows last installed updates, and driver updates
  include firmware, which on many brand PCs means the BIOS. Neither tool has a
  machine-readable count — winget prints a table in your own language and Windows Update answers only
  through COM — so both are read defensively and say nothing rather than guess. One Windows Update search
  serves both of its steps and the PC info tab.
- **Free space.** The Free up space step says how much room is left on the drive Windows is on, and
  stands out below 20 GB, about what Windows needs to install a new version of itself.
- **Last done and streaks.** Each step remembers the day it was last ticked and says so when it has been
  a while, and the header counts the weeks you have finished in a row.
- **Graphics drivers.** This step lists the PC's NVIDIA, AMD and Intel graphics with their installed
  drivers. For NVIDIA it looks up the latest driver online, through the undocumented lookup behind
  NVIDIA's driver search, and says whether an update is available; AMD and Intel offer no such lookup.
  Each card gets a button for NVIDIA App, AMD Software: Adrenalin Edition or Intel Graphics Software, or
  for the driver download page if that app isn't installed. The status updates when you come back to the
  window after installing a driver. Removing the step in **Settings** also stops the online lookup.
- **PC info.** A second tab lists what the PC is: maker and model, processor, memory, graphics, free
  space on the system drive, the Windows edition, version and build, the organization that manages the PC
  (an Active Directory domain or Microsoft Entra ID), the BIOS version and how old it is, the boot mode,
  Secure Boot and the TPM. It is read from the registry and Windows itself each time the tab opens, in a
  few milliseconds and without admin rights. **Copy to clipboard** puts it all in plain text, for example
  to ask someone for help.
  - *Is Windows up to date?* The tab answers the way Windows does: by asking Windows Update whether
    anything is waiting, and whether a restart is needed to finish installing updates. A newer version of
    Windows meant for the PC shows up there too.
  - *Installed on* is the day Windows was first installed on the PC. Windows' own Settings show the day of
    the last upgrade to a new version instead, which Windows also records as its install date.
  - *Is the BIOS up to date?* Nothing on the PC can tell, and every maker publishes BIOS versions
    differently, so the tab shows the BIOS's age and a **Find BIOS updates** button. It opens the support
    site of Dell, HP or Lenovo, which finds the PC's downloads by itself, Windows Update for Surface, or
    else a web search for the model's BIOS.
- **Repair Windows.** For when Windows misbehaves, **Settings** can run DISM and System File Checker
  (`sfc /scannow`) as administrator in a console window that stays open.
- **Starts with Windows, if you like.** On its first start the .exe asks whether to remind you every
  week. With a yes it registers a sign-in entry for your user and a daily Task Scheduler task of your own
  at the reminder hour (no admin rights needed), and re-registers them if the .exe has moved. Both run
  `UpdateCheck.exe --check`, which opens the window only while this week's checklist is unfinished, at
  most once a day, and not while a full-screen game or a presentation has the screen. The **Automatic
  reminder** switch in the footer turns this on and off.
- **Updates itself.** At most once a day the .exe asks GitHub for the latest release of
  `Zorakidd/Update-check` (`GITHUB_REPO` in [updater.py](src/update_check/updater.py)). If its tag is a
  newer version and an .exe is attached, the app downloads it, checks its size and the SHA-256 digest
  GitHub publishes for it — a download without one isn't installed — swaps it in for the running .exe and
  restarts with the same arguments. Failures are logged and never stop the app; a repository without any
  release yet is no failure. **Settings** shows the running version, and **Check now** asks straight away
  instead of waiting for the daily check. Asking only once a day keeps both the request and the updater's
  own imports out of nearly every start.

Steps and progress are stored in `%APPDATA%\UpdateCheck\state.json`, next to `update-check.log`. Set
`UPDATE_CHECK_DATA` to use a different state file.

## For IT

- **Policy.** Set the DWORD `DisableSelfUpdate` to `1` under `HKLM\SOFTWARE\Policies\UpdateCheck`, by Group
  Policy Preferences, an Intune script or any other way of setting a registry value, to roll out new
  versions yourself. The app then never contacts GitHub, and Settings says that your organization rolls out
  its new versions.
- **Rights.** Nothing needs administrator rights except Repair Windows, which asks through UAC. The
  reminder is a per-user sign-in entry (`HKCU\...\Run`) and a per-user Task Scheduler task,
  `UpdateCheck (<user name>)`, so several users on one PC each have their own.
- **Network.** The app itself contacts only GitHub, at most once a day for its own releases, and NVIDIA's
  driver search when the PC has NVIDIA graphics. winget and Windows Update reach their own servers. There
  is no telemetry.
- **Logs.** `%APPDATA%\UpdateCheck\update-check.log` holds warnings and unexpected errors with their
  tracebacks. It is kept to about 1 MB, with the previous log beside it as `update-check.log.1`.
- **Removal.** Turn off the **Automatic reminder** switch, which deletes the sign-in entry and the task,
  then delete the .exe and `%APPDATA%\UpdateCheck`.

## Develop

```
uv run update-check
uv run pytest
uvx ruff check .
```

Running from source never updates itself or registers with Windows startup on its own; the footer
switch still works.

The icon, [icon.ico](src/update_check/icon.ico), is drawn by
`uv run --no-project --with pillow python tools/make_icon.py`.

## Build and release

1. Bump `version` in `pyproject.toml`.
2. Build the single-file .exe from [UpdateCheck.spec](UpdateCheck.spec):

   ```
   uv run --with pyinstaller pyinstaller UpdateCheck.spec
   ```

   The spec holds the build's options: the package metadata, which the updater reads the running version
   from and stays off without, and the icon, on the .exe and taken along for the title bar and taskbar.
   Always build from the spec. Given `__main__.py` instead, PyInstaller writes a new spec over it from its
   command line alone and drops whatever that lacks.
3. Publish a GitHub release tagged `v<version>` (for example `v0.2.0`) with `dist/UpdateCheck.exe`
   attached. The repository has to be public, since the updater doesn't authenticate.
