"""What this PC is, for the PC info tab: maker and model, processor, memory, graphics, system drive, Windows,
who manages the PC, and its firmware.

Everything comes from the registry and a few Windows API calls, so reading it all takes a few milliseconds,
needs no administrator rights and flashes up no console. Whatever can't be read is left out rather than guessed.
"""

import ctypes
import os
import urllib.parse
import winreg
from ctypes import wintypes
from dataclasses import dataclass
from datetime import date, datetime
from typing import Any

from update_check import graphics
from update_check.system import size, system_drive_space


@dataclass(frozen=True)
class Section:
    """A card of the tab: its title, its (label, value) rows, a value maybe running over several lines, and
    perhaps a button under them."""

    title: str
    rows: list[tuple[str, str]]
    # The button's label, and the page or settings it opens.
    action: tuple[str, str] | None = None


_BIOS = r"HARDWARE\DESCRIPTION\System\BIOS"
_PROCESSOR = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"
_WINDOWS = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"
_SECURE_BOOT = r"SYSTEM\CurrentControlSet\Control\SecureBoot\State"
# Where Windows keeps, for each version it was upgraded from, a key named "Source OS (Updated on ...)".
_SETUP = r"SYSTEM\Setup"
# Either one is there while Windows waits for a restart to finish installing updates.
_RESTART_KEYS = (
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
)
# What PCs built from parts report instead of a maker or a model.
_PLACEHOLDERS = {
    "", "default string", "none", "not applicable", "o.e.m.", "system manufacturer", "system product name",
    "to be filled by o.e.m.",
}
# Makers whose support sites find the downloads for the PC they are opened on, by the start of their name.
_SUPPORT_SITES = {
    "DELL": "https://www.dell.com/support/home/",
    "HEWLETT-PACKARD": "https://support.hp.com/drivers",
    "HP": "https://support.hp.com/drivers",
    "LENOVO": "https://pcsupport.lenovo.com/",
    # Surface firmware comes through Windows Update, as does a Hyper-V virtual machine's.
    "MICROSOFT": "ms-settings:windowsupdate",
}
# The IMAGE_FILE_MACHINE values IsWow64Process2 reports for the processor.
_MACHINES = {0x014C: "x86", 0x8664: "x64", 0xAA64: "ARM64"}
_BOOT_MODES = {1: "Legacy BIOS", 2: "UEFI"}
_RELATION_PROCESSOR_CORE = 0
_TPM_NOT_FOUND = 0x8028400F
_TPM_VERSIONS = {1: "1.2", 2: "2.0"}
_NETSETUP_DOMAIN_NAME = 3
_DSREG_DEVICE_JOIN = 1

_kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
_kernel32.GetLogicalProcessorInformationEx.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD)]
_kernel32.GetPhysicallyInstalledSystemMemory.argtypes = [ctypes.POINTER(ctypes.c_ulonglong)]
_kernel32.GetFirmwareType.argtypes = [ctypes.POINTER(ctypes.c_int)]


class _TpmInfo(ctypes.Structure):
    _fields_ = [
        ("struct_version", ctypes.c_uint32), ("tpm_version", ctypes.c_uint32),
        ("interface_type", ctypes.c_uint32), ("revision", ctypes.c_uint32),
    ]


class _JoinInfo(ctypes.Structure):
    """The start of DSREG_JOIN_INFO, as far as the tenant's name."""

    _fields_ = [
        ("join_type", ctypes.c_int), ("certificate", ctypes.c_void_p), ("device_id", wintypes.LPWSTR),
        ("idp_domain", wintypes.LPWSTR), ("tenant_id", wintypes.LPWSTR), ("user_email", wintypes.LPWSTR),
        ("tenant_name", wintypes.LPWSTR),
    ]


def gather(updates: str = "", today: date | None = None) -> list[Section]:
    """Every section of the tab, read now. `updates` is what Windows Update said, which takes too long to ask
    here."""
    today = today or date.today()
    bios, windows = _values(_BIOS), _values(_WINDOWS)
    build = _text(windows, "CurrentBuild")
    sections = [
        Section("Device", [
            ("Name", os.environ.get("COMPUTERNAME", "")),
            ("Model", model(bios)),
            ("Processor", _processor()),
            ("Memory", _memory()),
            ("Graphics", "\n".join(_adapters())),
            ("System drive", _system_drive()),
        ]),
        Section("Windows", [
            ("Edition", windows_name(_text(windows, "ProductName"), int(build) if build.isdigit() else 0)),
            ("Version", _text(windows, "DisplayVersion") or _text(windows, "ReleaseId")),
            ("OS build", f"{build}.{windows['UBR']}" if build and isinstance(windows.get("UBR"), int) else build),
            ("Updates", updates),
            ("Installed on", _installed(first_installed(windows.get("InstallDate"), _earlier_installs()))),
            ("System type", _system_type()),
            ("Managed by", _managed_by()),
        ]),
        Section("Firmware", [
            ("BIOS", bios_version(bios, today)),
            ("Boot mode", _BOOT_MODES.get(_firmware_type(), "")),
            ("Secure Boot", _secure_boot()),
            ("TPM", _tpm()),
        ], action=("Find BIOS updates", bios_page(bios))),
    ]
    # Whatever couldn't be read is left out, and so is a section with nothing left in it.
    read = [Section(section.title, [row for row in section.rows if row[1]], section.action) for section in sections]
    return [section for section in read if section.rows]


def as_text(sections: list[Section]) -> str:
    """The sections as plain text, for the clipboard."""
    return "\n\n".join(
        "\n".join([section.title, *(f"{label}: {value.replace('\n', ', ')}" for label, value in section.rows)])
        for section in sections
    )


def restart_pending() -> bool:
    """Whether Windows waits for a restart to finish installing updates."""
    for path in _RESTART_KEYS:
        try:
            winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, path, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY).Close()
        except OSError:
            continue
        return True
    return False


# --- Formatting --------------------------------------------------------------------------------------------

def model(bios: dict[str, Any]) -> str:
    """The PC's maker and model. A PC built from parts names its mainboard instead, and Lenovo keeps the model
    in the version, its product name being an order number."""
    maker, product = _named(bios, "SystemManufacturer"), _named(bios, "SystemProductName")
    if maker.upper() == "LENOVO":
        product = _named(bios, "SystemVersion") or product
    if not product:
        maker, product = _named(bios, "BaseBoardManufacturer"), _named(bios, "BaseBoardProduct")
    # HP's product names already start with "HP".
    if product.lower().startswith(maker.lower()):
        return product
    return f"{maker} {product}".strip()


def windows_name(product: str, build: int) -> str:
    """Windows 11 still calls itself Windows 10 in the registry; only its build number, 22000 or later, tells."""
    if build >= 22000 and product.startswith("Windows 10"):
        return "Windows 11" + product.removeprefix("Windows 10")
    return product


def bios_version(bios: dict[str, Any], today: date) -> str:
    """The BIOS version, the day it was released and how long ago that was. The BIOS gives the day as
    month/day/year."""
    version, released = _text(bios, "BIOSVersion"), _text(bios, "BIOSReleaseDate")
    try:
        month, day, year = (int(part) for part in released.split("/"))
        day_released = date(year, month, day)
    except ValueError:
        pass
    else:
        released = f"{_date(day_released)} ({age(day_released, today)})"
    return " · ".join(part for part in (version, released) if part)


def age(day: date, today: date) -> str:
    """How old something from `day` is, in months, or in years from two years on."""
    months = (today.year - day.year) * 12 + today.month - day.month - (today.day < day.day)
    if months < 1:
        return "less than a month old"
    if months < 24:
        return f"{months} month{'s' if months > 1 else ''} old"
    return f"{months // 12} years old"


def bios_page(bios: dict[str, Any]) -> str:
    """Where to look for a newer BIOS. Neither Windows nor the BIOS can tell whether there is one, and every
    maker publishes them differently: so this is a support site that finds the PC's downloads by itself, or
    else a web search for the model's BIOS."""
    maker = _named(bios, "SystemManufacturer").upper()
    for name, site in _SUPPORT_SITES.items():
        if maker.startswith(name):
            return site
    return "https://www.bing.com/search?q=" + urllib.parse.quote_plus(f"{model(bios)} BIOS update".strip())


def management(domain: str | None, tenant: str | None) -> str:
    """Who manages the PC: an Active Directory domain, a Microsoft Entra ID tenant ("" for one without a name),
    both, or nobody."""
    parts = []
    if tenant is not None:
        parts.append(f"{tenant} (Microsoft Entra ID)" if tenant else "Microsoft Entra ID")
    if domain:
        parts.append(f"{domain} (Active Directory)")
    return "\n".join(parts) or "Not managed by an organization"


def first_installed(current: Any, earlier: list[Any]) -> int | None:
    """When Windows was first installed on this PC, in seconds since 1970. Windows moves its install date to
    the day of each upgrade to a new version, so the earliest of the current and the earlier versions' dates
    wins."""
    stamps = [stamp for stamp in (current, *earlier) if isinstance(stamp, int) and stamp > 0]
    return min(stamps, default=None)


def _date(day: date) -> str:
    return f"{day:%b} {day.day}, {day.year}"


# --- Reading -----------------------------------------------------------------------------------------------

def _values(path: str) -> dict[str, Any]:
    """The values of an HKEY_LOCAL_MACHINE key by name, or none if the key can't be read."""
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, path, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
            entries = (winreg.EnumValue(key, index) for index in range(winreg.QueryInfoKey(key)[1]))
            return {name: value for name, value, _ in entries}
    except OSError:
        return {}


def _earlier_installs() -> list[Any]:
    """The install dates of the versions of Windows this PC was upgraded from."""
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, _SETUP, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
            names = [winreg.EnumKey(key, index) for index in range(winreg.QueryInfoKey(key)[0])]
    except OSError:
        return []
    return [_values(rf"{_SETUP}\{name}").get("InstallDate") for name in names if name.startswith("Source OS")]


def _text(values: dict[str, Any], name: str) -> str:
    value = values.get(name)
    # Processor names come padded with spaces.
    return " ".join(value.split()) if isinstance(value, str) else ""


def _named(values: dict[str, Any], name: str) -> str:
    text = _text(values, name)
    return "" if text.lower() in _PLACEHOLDERS else text


def _processor() -> str:
    name = _text(_values(_PROCESSOR), "ProcessorNameString")
    cores, threads = _cores(), os.cpu_count()
    if not cores or not threads:
        return name
    count = f"{cores} cores, {threads} threads" if threads != cores else f"{cores} cores"
    return f"{name}\n{count}" if name else count


def _cores() -> int:
    """How many cores the processor has, or 0 if Windows won't say."""
    length = wintypes.DWORD()
    _kernel32.GetLogicalProcessorInformationEx(_RELATION_PROCESSOR_CORE, None, ctypes.byref(length))
    buffer = ctypes.create_string_buffer(length.value)
    if not length.value or not _kernel32.GetLogicalProcessorInformationEx(
        _RELATION_PROCESSOR_CORE, buffer, ctypes.byref(length),
    ):
        return 0
    # One record per core, each starting with its relationship and then its own size in bytes.
    raw, offset, cores = buffer.raw, 0, 0
    while offset < length.value:
        record = int.from_bytes(raw[offset + 4:offset + 8], "little")
        if not record:
            return 0
        offset += record
        cores += 1
    return cores


def _memory() -> str:
    kilobytes = ctypes.c_ulonglong()
    if not _kernel32.GetPhysicallyInstalledSystemMemory(ctypes.byref(kilobytes)):
        return ""
    return size(kilobytes.value * 1024)


def _adapters() -> list[str]:
    try:
        return graphics.adapter_names()
    except OSError:
        return []


def _system_drive() -> str:
    space = system_drive_space()
    if space is None:
        return ""
    drive, free, total = space
    return f"{size(free)} free of {size(total)} ({drive})"


def _installed(timestamp: Any) -> str:
    """A day from seconds since 1970, as the tab shows it."""
    if not isinstance(timestamp, int) or timestamp <= 0:
        return ""
    try:
        return _date(datetime.fromtimestamp(timestamp).date())
    except (OverflowError, OSError, ValueError):
        return ""


def _system_type() -> str:
    """32- or 64-bit Windows and the processor's architecture, which tells an ARM PC from the rest. Asked of
    Windows itself, since an x64 program running on an ARM PC is told it's on x64."""
    check = getattr(_kernel32, "IsWow64Process2", None)
    if check is None:
        return ""
    check.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.USHORT), ctypes.POINTER(wintypes.USHORT)]
    process, native = wintypes.USHORT(), wintypes.USHORT()
    # -1 stands for the current process.
    if not check(wintypes.HANDLE(-1), ctypes.byref(process), ctypes.byref(native)):
        return ""
    machine = _MACHINES.get(native.value)
    if machine is None:
        return ""
    return f"{'32' if machine == 'x86' else '64'}-bit Windows, {machine} processor"


def _managed_by() -> str:
    try:
        netapi = ctypes.WinDLL("netapi32")
    except OSError:
        return ""
    return management(_domain(netapi), _entra_tenant(netapi))


def _domain(netapi: ctypes.WinDLL) -> str | None:
    """The Active Directory domain the PC is joined to, if any. A workgroup isn't one."""
    name, status = wintypes.LPWSTR(), ctypes.c_int()
    if netapi.NetGetJoinInformation(None, ctypes.byref(name), ctypes.byref(status)) != 0:
        return None
    try:
        return name.value if status.value == _NETSETUP_DOMAIN_NAME else None
    finally:
        netapi.NetApiBufferFree(name)


def _entra_tenant(netapi: ctypes.WinDLL) -> str | None:
    """The name of the Microsoft Entra ID tenant the PC is joined to: "" for a tenant without one, None if it
    isn't joined. A work account merely added to a personal PC doesn't put the organization in charge of it."""
    function = getattr(netapi, "NetGetAadJoinInformation", None)
    if function is None:
        return None
    function.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(ctypes.POINTER(_JoinInfo))]
    info = ctypes.POINTER(_JoinInfo)()
    if function(None, ctypes.byref(info)) != 0 or not info:
        return None
    try:
        return (info.contents.tenant_name or "") if info.contents.join_type == _DSREG_DEVICE_JOIN else None
    finally:
        netapi.NetFreeAadJoinInformation(info)


def _firmware_type() -> int:
    kind = ctypes.c_int()
    return kind.value if _kernel32.GetFirmwareType(ctypes.byref(kind)) else 0


def _secure_boot() -> str:
    enabled = _values(_SECURE_BOOT).get("UEFISecureBootEnabled")
    if not isinstance(enabled, int):
        # The key is missing where the firmware has no Secure Boot, as with a legacy BIOS.
        return "Not supported"
    return "On" if enabled else "Off"


def _tpm() -> str:
    """The TPM's version, which Windows 11 needs to be 2.0. TPM Base Services tell anyone; Get-Tpm needs an
    administrator."""
    try:
        tbs = ctypes.WinDLL("tbs")
    except OSError:
        return ""
    tbs.Tbsi_GetDeviceInfo.restype = ctypes.c_uint32
    info = _TpmInfo()
    result = tbs.Tbsi_GetDeviceInfo(ctypes.sizeof(info), ctypes.byref(info))
    if result == _TPM_NOT_FOUND:
        return "Not found, or turned off in the BIOS"
    return _TPM_VERSIONS.get(info.tpm_version, "") if result == 0 else ""
