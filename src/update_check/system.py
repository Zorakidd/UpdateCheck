"""Windows integration: automatic reminders, step actions and single-instance handling."""

import ctypes
import os
import re
import subprocess
import sys
import winreg
from ctypes import wintypes
from pathlib import Path

APP_NAME = "UpdateCheck"
# The window title, which a second copy of the app also uses to find and focus the open window.
TITLE = "Update Check"
# How the app names itself to the servers it asks about updates: GitHub's releases and NVIDIA's driver search.
USER_AGENT = "Update-Check"
# True when running as the PyInstaller-built .exe rather than from source.
FROZEN = getattr(sys, "frozen", False)
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
# Where Group Policy or Intune put an administrator's settings for this app.
POLICY_KEY = r"SOFTWARE\Policies\UpdateCheck"
# The daily task's name before each user got one of their own. Task names are shared by everyone on the PC.
_SHARED_TASK = APP_NAME
# A URI scheme such as "ms-settings:" or "https:". Two or more letters, so "C:\..." is not a URI.
_URI = re.compile(r"^[A-Za-z][A-Za-z0-9+.\-]+:")
_ERROR_ALREADY_EXISTS = 183
_ERROR_CANCELLED = 1223
_SW_RESTORE = 9
_SPI_GETWORKAREA = 0x30
# What SHQueryUserNotificationState says while a full-screen app or game, or a presentation, has the screen.
_QUNS_BUSY, _QUNS_RUNNING_D3D_FULL_SCREEN, _QUNS_PRESENTATION_MODE = 2, 3, 4

_kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
_kernel32.CreateMutexW.restype = wintypes.HANDLE
_kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
_kernel32.GetDiskFreeSpaceExW.argtypes = [
    wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_ulonglong), ctypes.POINTER(ctypes.c_ulonglong), ctypes.c_void_p,
]


def policy(name: str) -> int | None:
    """An administrator's setting for this app, from Group Policy or Intune, or None if there is none."""
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, POLICY_KEY) as key:
            value, kind = winreg.QueryValueEx(key, name)
    except OSError:
        return None
    return value if kind == winreg.REG_DWORD else None


def self_update_allowed() -> bool:
    """False where an administrator has set DisableSelfUpdate, to roll out new versions themselves."""
    return policy("DisableSelfUpdate") != 1


def system_drive_space() -> tuple[str, int, int] | None:
    """The drive Windows is on, which needn't be C:, with its free and total bytes."""
    drive = os.environ.get("SystemDrive") or "C:"
    free, total = ctypes.c_ulonglong(), ctypes.c_ulonglong()
    if not _kernel32.GetDiskFreeSpaceExW(drive + "\\", ctypes.byref(free), ctypes.byref(total), None):
        return None
    return drive, free.value, total.value


def size(count: int) -> str:
    """A size in bytes as GB, or as TB from 1000 GB, to three figures. Counted in 1024s, like Windows does."""
    value, unit = count / 1024**3, "GB"
    if value >= 1000:
        value, unit = value / 1024, "TB"
    text = f"{value:.{2 if value < 10 else 1 if value < 100 else 0}f}"
    return f"{text.rstrip('0').rstrip('.') if '.' in text else text} {unit}"


def reminder_command() -> list[str]:
    """The command that runs a background reminder check without a console window."""
    if FROZEN:
        return [sys.executable, "--check"]
    python = Path(sys.executable)
    pythonw = python.with_name("pythonw.exe")
    return [str(pythonw if pythonw.exists() else python), "-m", "update_check", "--check"]


def _registered_command() -> str | None:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
            return winreg.QueryValueEx(key, APP_NAME)[0]
    except FileNotFoundError:
        return None


def autostart_enabled() -> bool:
    return _registered_command() is not None


def sync_autostart(enabled: bool, hour: int) -> None:
    """Make the registration match the setting, and re-register if the .exe has moved."""
    if not enabled:
        if autostart_enabled():
            disable_autostart()
    elif _registered_command() != subprocess.list2cmdline(reminder_command()):
        enable_autostart(hour)


def task_name() -> str:
    """The daily task's name, one per user: a second user on the PC could neither create nor replace a task
    that another one owns."""
    user = re.sub(r'[\\/:*?"<>|]', "_", os.environ.get("USERNAME") or "user")
    return f"{APP_NAME} ({user})"


def enable_autostart(hour: int) -> None:
    """Check for a due reminder at every sign-in and daily at the reminder hour. No admin rights needed."""
    command = subprocess.list2cmdline(reminder_command())
    _schtasks("/Create", "/F", "/SC", "DAILY", "/ST", f"{hour:02d}:00", "/TN", task_name(), "/TR", command)
    # Replaced by this user's own task. Only its owner may delete it, so another user's is left alone.
    _schtasks("/Delete", "/F", "/TN", _SHARED_TASK, check=False)
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
        winreg.SetValueEx(key, APP_NAME, 0, winreg.REG_SZ, command)


def disable_autostart() -> None:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, access=winreg.KEY_SET_VALUE) as key:
            winreg.DeleteValue(key, APP_NAME)
    except FileNotFoundError:
        pass
    for name in (task_name(), _SHARED_TASK):
        _schtasks("/Delete", "/F", "/TN", name, check=False)


def _schtasks(*args: str, check: bool = True) -> None:
    try:
        result = subprocess.run(
            ["schtasks", *args],
            check=False,
            capture_output=True,
            encoding="oem",
            errors="replace",
            creationflags=subprocess.CREATE_NO_WINDOW,
            # Task Scheduler answers in well under a second; this only keeps a hung one from hanging the app.
            timeout=30,
        )
    except subprocess.TimeoutExpired as error:
        if check:
            raise OSError("Task Scheduler didn't answer") from error
        return
    if check and result.returncode:
        raise OSError(result.stderr.strip() or f"schtasks exited with code {result.returncode}")


def open_action(action: str) -> None:
    """Open a URI, settings page or file; run anything else as a command in a new terminal."""
    if _URI.match(action) or Path(action).exists():
        os.startfile(action)
    else:
        subprocess.Popen(["cmd", "/k", action], creationflags=subprocess.CREATE_NEW_CONSOLE)


# DISM goes first: it repairs the Windows image that System File Checker restores files from.
_REPAIR_WINDOWS = (
    "title Repair Windows"
    " & echo Step 1 of 2: Repairing the Windows image with DISM..."
    " & DISM /Online /Cleanup-Image /RestoreHealth"
    " & echo."
    " & echo Step 2 of 2: Checking system files with System File Checker..."
    " & sfc /scannow"
    " & echo."
    " & echo Finished. If files were repaired, restart the PC."
)


def repair_windows() -> None:
    """Run DISM and System File Checker as administrator, in a console window that stays open."""
    try:
        os.startfile("cmd.exe", "runas", f"/k {_REPAIR_WINDOWS}")
    except OSError as error:
        # Declining the administrator prompt isn't an error.
        if error.winerror != _ERROR_CANCELLED:
            raise


def user_is_busy() -> bool:
    """Whether a full-screen game or app, or a presentation, has the screen, so a reminder shouldn't pop up."""
    state = ctypes.c_int()
    # shell32 is loaded only here, since most background checks have no reminder to show.
    if ctypes.windll.shell32.SHQueryUserNotificationState(ctypes.byref(state)) != 0:
        return False
    return state.value in (_QUNS_BUSY, _QUNS_RUNNING_D3D_FULL_SCREEN, _QUNS_PRESENTATION_MODE)


def claim_single_instance() -> int | None:
    """Return a lock that stays held until released or the process exits, or None if another
    Update Check process already holds it."""
    handle = _kernel32.CreateMutexW(None, False, f"Local\\{APP_NAME}.SingleInstance")
    if ctypes.get_last_error() == _ERROR_ALREADY_EXISTS:
        _kernel32.CloseHandle(handle)
        return None
    return handle


def release_single_instance(lock: int) -> None:
    _kernel32.CloseHandle(lock)


def work_area() -> tuple[int, int, int, int]:
    """The primary screen without the taskbar as (left, top, right, bottom), in physical pixels
    once CustomTkinter has made the process DPI aware."""
    rect = wintypes.RECT()
    ctypes.windll.user32.SystemParametersInfoW(_SPI_GETWORKAREA, 0, ctypes.byref(rect), 0)
    return rect.left, rect.top, rect.right, rect.bottom


def focus_window(title: str) -> None:
    user32 = ctypes.WinDLL("user32")
    if hwnd := user32.FindWindowW(None, title):
        if user32.IsIconic(hwnd):
            user32.ShowWindow(hwnd, _SW_RESTORE)
        user32.SetForegroundWindow(hwnd)
