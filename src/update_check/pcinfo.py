"""What this PC is, for the PC info tab: maker and model, processor, memory, graphics, system drive, Windows
and firmware.

Everything comes from the registry and a few Windows API calls, so reading it all takes a few milliseconds,
needs no administrator rights and flashes up no console. Whatever can't be read is left out rather than guessed.
"""

import ctypes
import os
import winreg
from ctypes import wintypes
from datetime import date, datetime
from typing import Any

from update_check import graphics

# A section of the tab: its title and its (label, value) rows. A value may run over several lines.
type Section = tuple[str, list[tuple[str, str]]]

_BIOS = r"HARDWARE\DESCRIPTION\System\BIOS"
_PROCESSOR = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"
_WINDOWS = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"
_SECURE_BOOT = r"SYSTEM\CurrentControlSet\Control\SecureBoot\State"
# What PCs built from parts report instead of a maker or a model.
_PLACEHOLDERS = {
    "", "default string", "none", "not applicable", "o.e.m.", "system manufacturer", "system product name",
    "to be filled by o.e.m.",
}
# The IMAGE_FILE_MACHINE values IsWow64Process2 reports for the processor.
_MACHINES = {0x014C: "x86", 0x8664: "x64", 0xAA64: "ARM64"}
_BOOT_MODES = {1: "Legacy BIOS", 2: "UEFI"}
_RELATION_PROCESSOR_CORE = 0
_TPM_NOT_FOUND = 0x8028400F
_TPM_VERSIONS = {1: "1.2", 2: "2.0"}

_kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
_kernel32.GetLogicalProcessorInformationEx.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD)]
_kernel32.GetPhysicallyInstalledSystemMemory.argtypes = [ctypes.POINTER(ctypes.c_ulonglong)]
_kernel32.GetDiskFreeSpaceExW.argtypes = [
    wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_ulonglong), ctypes.POINTER(ctypes.c_ulonglong), ctypes.c_void_p,
]
_kernel32.GetFirmwareType.argtypes = [ctypes.POINTER(ctypes.c_int)]


class _TpmInfo(ctypes.Structure):
    _fields_ = [
        ("struct_version", ctypes.c_uint32), ("tpm_version", ctypes.c_uint32),
        ("interface_type", ctypes.c_uint32), ("revision", ctypes.c_uint32),
    ]


def gather() -> list[Section]:
    """Every section of the tab, read now."""
    bios, windows = _values(_BIOS), _values(_WINDOWS)
    build = _text(windows, "CurrentBuild")
    sections: list[Section] = [
        ("Device", [
            ("Name", os.environ.get("COMPUTERNAME", "")),
            ("Model", model(bios)),
            ("Processor", _processor()),
            ("Memory", _memory()),
            ("Graphics", "\n".join(_adapters())),
            ("System drive", _system_drive()),
        ]),
        ("Windows", [
            ("Edition", windows_name(_text(windows, "ProductName"), int(build) if build.isdigit() else 0)),
            ("Version", _text(windows, "DisplayVersion") or _text(windows, "ReleaseId")),
            ("OS build", f"{build}.{windows['UBR']}" if build and isinstance(windows.get("UBR"), int) else build),
            ("Installed on", _installed(windows.get("InstallDate"))),
            ("System type", _system_type()),
        ]),
        ("Firmware", [
            ("BIOS", bios_version(bios)),
            ("Boot mode", _BOOT_MODES.get(_firmware_type(), "")),
            ("Secure Boot", _secure_boot()),
            ("TPM", _tpm()),
        ]),
    ]
    # Whatever couldn't be read is left out, and so is a section with nothing left in it.
    read = [(title, [(label, value) for label, value in rows if value]) for title, rows in sections]
    return [(title, rows) for title, rows in read if rows]


def as_text(sections: list[Section]) -> str:
    """The sections as plain text, for the clipboard."""
    return "\n\n".join(
        "\n".join([title, *(f"{label}: {value.replace('\n', ', ')}" for label, value in rows)])
        for title, rows in sections
    )


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


def bios_version(bios: dict[str, Any]) -> str:
    """The BIOS version and the day it was released, which the BIOS reports as month/day/year."""
    released = _text(bios, "BIOSReleaseDate")
    try:
        month, day, year = (int(part) for part in released.split("/"))
        released = _date(date(year, month, day))
    except ValueError:
        pass
    return " · ".join(part for part in (_text(bios, "BIOSVersion"), released) if part)


def size(count: int) -> str:
    """A size in bytes as GB, or as TB from 1000 GB, to three figures. Counted in 1024s, like Windows does."""
    value, unit = count / 1024**3, "GB"
    if value >= 1000:
        value, unit = value / 1024, "TB"
    text = f"{value:.{2 if value < 10 else 1 if value < 100 else 0}f}"
    return f"{text.rstrip('0').rstrip('.') if '.' in text else text} {unit}"


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
    # Windows needn't be on C:.
    drive = os.environ.get("SystemDrive") or "C:"
    free, total = ctypes.c_ulonglong(), ctypes.c_ulonglong()
    if not _kernel32.GetDiskFreeSpaceExW(drive + "\\", ctypes.byref(free), ctypes.byref(total), None):
        return ""
    return f"{size(free.value)} free of {size(total.value)} ({drive})"


def _installed(timestamp: Any) -> str:
    """The day Windows was installed, or last upgraded to a new version, from seconds since 1970."""
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
