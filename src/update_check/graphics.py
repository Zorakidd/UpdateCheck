"""Graphics cards and their drivers: detection, the latest NVIDIA driver and the vendors' own apps.

Only NVIDIA can be asked for its latest driver, through the lookup its driver search page uses. That lookup
isn't documented, so anything unexpected counts as a failed check. AMD and Intel have nothing a program can
query (Intel's download pages refuse non-browser clients), so for their cards the vendor's own app or driver
download page takes over.
"""

import ctypes
import itertools
import json
import logging
import os
import queue
import re
import threading
import time
import urllib.parse
import urllib.request
import winreg
import xml.etree.ElementTree as ET
from ctypes import wintypes
from dataclasses import dataclass
from pathlib import Path

from update_check.updater import USER_AGENT

log = logging.getLogger(__name__)

NVIDIA, AMD, INTEL = "NVIDIA", "AMD", "Intel"
# By PCI vendor ID, in the order cards are listed: dedicated graphics usually come first.
_VENDORS = {"10DE": NVIDIA, "1002": AMD, "8086": INTEL}

# What is known about a card's driver.
CHECKING = "checking"
UP_TO_DATE = "up to date"
UPDATE_AVAILABLE = "update available"
CHECK_FAILED = "check failed"
NOT_CHECKED = "not checked"
NO_DRIVER = "no driver"

DOWNLOAD_PAGES = {
    NVIDIA: "https://www.nvidia.com/en-us/drivers/",
    AMD: "https://www.amd.com/en/support/download/drivers.html",
    # Intel Driver & Support Assistant, which finds the right driver for the PC.
    INTEL: "https://www.intel.com/content/www/us/en/support/detect.html",
}

_NVIDIA_PRODUCTS = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3"
# osID 57 is 64-bit Windows 10 and 11; dch=1 is the driver type those use.
_NVIDIA_DRIVERS = (
    "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php"
    "?func=DriverManualLookup&pfid={}&osID=57&languageCode=1033&isWHQL=1&dch=1&sort1=0&numberOfResults=1"
)


@dataclass(frozen=True)
class Card:
    vendor: str
    name: str
    # The version Windows reports, e.g. "32.0.16.1692". Empty when only Windows' basic display driver is installed.
    driver_version: str

    @property
    def version(self) -> str:
        """The driver version as the vendor names it: NVIDIA calls "32.0.16.1692" "616.92"."""
        digits = "".join(self.driver_version.split(".")[-2:])[-5:]
        if self.vendor == NVIDIA and len(digits) == 5 and digits.isdigit():
            return f"{digits[:3]}.{digits[3:]}"
        return self.driver_version


@dataclass(frozen=True)
class Release:
    version: str
    # The driver's download page.
    url: str


def is_newer(release: Release, card: Card) -> bool:
    return _numbers(release.version) > _numbers(card.version)


def _numbers(version: str) -> tuple[int, ...]:
    return tuple(int(part) for part in re.findall(r"\d+", version))


# --- Detection ---------------------------------------------------------------------------------------------

class _Guid(ctypes.Structure):
    _fields_ = [
        ("data1", wintypes.DWORD), ("data2", wintypes.WORD), ("data3", wintypes.WORD), ("data4", ctypes.c_ubyte * 8),
    ]


class _DeviceInfo(ctypes.Structure):
    _fields_ = [
        ("size", wintypes.DWORD), ("class_guid", _Guid), ("instance", wintypes.DWORD), ("reserved", ctypes.c_void_p),
    ]


_DISPLAY_CLASS = _Guid(0x4D36E968, 0xE325, 0x11CE, (ctypes.c_ubyte * 8)(0xBF, 0xC1, 0x08, 0x00, 0x2B, 0xE1, 0x03, 0x18))
_DIGCF_PRESENT = 0x2
_SPDRP_HARDWAREID = 0x1
_SPDRP_DRIVER = 0x9
_INVALID_HANDLE = ctypes.c_void_p(-1).value

_setupapi = ctypes.WinDLL("setupapi", use_last_error=True)
_setupapi.SetupDiGetClassDevsW.argtypes = [ctypes.POINTER(_Guid), wintypes.LPCWSTR, wintypes.HWND, wintypes.DWORD]
_setupapi.SetupDiGetClassDevsW.restype = wintypes.HANDLE
_setupapi.SetupDiEnumDeviceInfo.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(_DeviceInfo)]
_setupapi.SetupDiGetDeviceRegistryPropertyW.argtypes = [
    wintypes.HANDLE, ctypes.POINTER(_DeviceInfo), wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
    ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
]
_setupapi.SetupDiDestroyDeviceInfoList.argtypes = [wintypes.HANDLE]


def detect() -> list[Card]:
    """The NVIDIA, AMD and Intel graphics cards in this PC. Takes a few milliseconds."""
    devices = _setupapi.SetupDiGetClassDevsW(ctypes.byref(_DISPLAY_CLASS), None, None, _DIGCF_PRESENT)
    if devices in (None, _INVALID_HANDLE):
        raise ctypes.WinError(ctypes.get_last_error())
    cards = set()
    try:
        info = _DeviceInfo(size=ctypes.sizeof(_DeviceInfo))
        for index in itertools.count():
            if not _setupapi.SetupDiEnumDeviceInfo(devices, index, ctypes.byref(info)):
                break
            if card := _card(devices, info):
                cards.add(card)
    finally:
        _setupapi.SetupDiDestroyDeviceInfoList(devices)
    order = list(_VENDORS.values())
    return sorted(cards, key=lambda card: (order.index(card.vendor), card.name))


def vendor_of(hardware_id: str) -> str | None:
    """The vendor from a hardware ID such as PCI\\VEN_10DE&DEV_2820."""
    match = re.search(r"VEN_([0-9A-F]{4})", hardware_id, re.IGNORECASE)
    return _VENDORS.get(match.group(1).upper()) if match else None


def _card(devices, info: _DeviceInfo) -> Card | None:
    vendor = vendor_of(_device_property(devices, info, _SPDRP_HARDWAREID))
    driver_key = _device_property(devices, info, _SPDRP_DRIVER)
    if vendor is None or not driver_key:
        return None
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, rf"SYSTEM\CurrentControlSet\Control\Class\{driver_key}") as key:
            if winreg.QueryValueEx(key, "ProviderName")[0] == "Microsoft":
                # Windows' basic display driver stands in while the vendor's driver is missing.
                return Card(vendor, f"{vendor} graphics", "")
            return Card(vendor, winreg.QueryValueEx(key, "DriverDesc")[0], winreg.QueryValueEx(key, "DriverVersion")[0])
    except OSError:
        return None


def _device_property(devices, info: _DeviceInfo, prop: int) -> str:
    buffer = ctypes.create_unicode_buffer(1024)
    found = _setupapi.SetupDiGetDeviceRegistryPropertyW(
        devices, ctypes.byref(info), prop, None, buffer, ctypes.sizeof(buffer), None,
    )
    # A list of hardware IDs comes back NUL-separated; `value` is the first and most specific.
    return buffer.value if found else ""


# --- The latest NVIDIA driver ------------------------------------------------------------------------------

def latest_nvidia_driver(card: Card) -> Release | None:
    """The newest driver for the card, or None if NVIDIA's driver search doesn't list the card."""
    product = nvidia_product_id(_download(_NVIDIA_PRODUCTS), card.name)
    return parse_nvidia_driver(_download(_NVIDIA_DRIVERS.format(product))) if product else None


def nvidia_product_id(products_xml: bytes, name: str) -> str | None:
    """Find a card in NVIDIA's product list, whose names may lack the "NVIDIA" prefix and memory size."""
    products: dict[str, str] = {}
    for product in ET.fromstring(products_xml).iter("LookupValue"):
        # Some cards are listed twice, as desktop and notebook products; both get the same driver version.
        products.setdefault(_product_key(product.findtext("Name", "")), product.findtext("Value", ""))
    key = _product_key(name)
    return products.get(key) or products.get(re.sub(r" \d+gb$", "", key))


def _product_key(name: str) -> str:
    return " ".join(re.sub(r"\((?:r|tm)\)|[®™]|^nvidia ", " ", name.lower()).split())


def parse_nvidia_driver(response: bytes) -> Release | None:
    drivers = json.loads(response).get("IDS") or []
    if not drivers:
        return None
    info = drivers[0]["downloadInfo"]
    version = info["Version"]
    if not re.fullmatch(r"\d+\.\d+", version):
        raise ValueError(f"Unexpected NVIDIA driver version {version!r}")
    return Release(version, urllib.parse.unquote(info.get("DetailsURL") or "") or DOWNLOAD_PAGES[NVIDIA])


def _download(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=15) as response:
        return response.read()


# --- Lookups in the background -----------------------------------------------------------------------------

@dataclass(frozen=True)
class Lookup:
    release: Release | None = None
    failed: bool = False


class DriverChecks:
    """The PC's graphics cards and their latest drivers, looked up in background threads.

    `refresh()` detects the cards and starts lookups; `collect()`, called from the window's thread, takes in
    the finished ones.
    """

    # Seconds before a failed lookup is tried again, e.g. once the PC is back online.
    RETRY_AFTER = 60

    def __init__(self, detect=detect, look_up=latest_nvidia_driver):
        self.cards: list[Card] = []
        self._detect = detect
        self._look_up = look_up
        # Finished lookups by card name. The latest driver for a card doesn't change when its driver is updated.
        self._lookups: dict[str, Lookup] = {}
        self._failed_at: dict[str, float] = {}
        self._running: set[str] = set()
        self._finished: queue.SimpleQueue[tuple[str, Lookup]] = queue.SimpleQueue()

    @property
    def busy(self) -> bool:
        return bool(self._running)

    def refresh(self, recheck: bool = False) -> bool:
        """Detect the cards and look up drivers that haven't been, whose lookup failed a while ago, or all of
        them with `recheck`. Returns True if anything shown about the cards changed."""
        try:
            cards = self._detect()
        except OSError:
            log.exception("Couldn't detect the graphics cards")
            cards = []
        changed = cards != self.cards
        self.cards = cards
        for card in cards:
            if card.vendor != NVIDIA or not card.driver_version or card.name in self._running:
                continue
            lookup = self._lookups.get(card.name)
            retry = lookup is not None and lookup.failed and time.monotonic() - self._failed_at[card.name] >= self.RETRY_AFTER
            if recheck or lookup is None or retry:
                self._lookups.pop(card.name, None)
                self._running.add(card.name)
                threading.Thread(target=self._run, args=(card,), daemon=True).start()
                changed = True
        return changed

    def collect(self) -> bool:
        """Take in finished lookups. Returns True if there were any."""
        collected = False
        while not self._finished.empty():
            name, lookup = self._finished.get()
            self._running.discard(name)
            self._lookups[name] = lookup
            if lookup.failed:
                self._failed_at[name] = time.monotonic()
            collected = True
        return collected

    def status(self, card: Card) -> tuple[str, Release | None]:
        if not card.driver_version:
            return NO_DRIVER, None
        if card.vendor != NVIDIA:
            return NOT_CHECKED, None
        lookup = self._lookups.get(card.name)
        if lookup is None:
            return CHECKING, None
        if lookup.failed:
            return CHECK_FAILED, None
        if lookup.release is None:
            return NOT_CHECKED, None
        return (UPDATE_AVAILABLE if is_newer(lookup.release, card) else UP_TO_DATE), lookup.release

    def _run(self, card: Card) -> None:
        try:
            lookup = Lookup(self._look_up(card))
        except Exception:
            # NVIDIA's lookup isn't documented and may change: whatever goes wrong, the check just failed.
            log.warning("Couldn't look up the latest driver for %s", card.name, exc_info=True)
            lookup = Lookup(failed=True)
        self._finished.put((card.name, lookup))


# --- The vendors' apps -------------------------------------------------------------------------------------

@dataclass(frozen=True)
class VendorApp:
    name: str
    exe: str

    def open(self) -> None:
        os.startfile(self.exe)


def vendor_app(vendor: str) -> VendorApp | None:
    """The vendor's graphics app, if it's installed."""
    name, exe = {
        NVIDIA: ("NVIDIA App", r"NVIDIA Corporation\NVIDIA App\CEF\NVIDIA App.exe"),
        AMD: ("AMD Software", r"AMD\CNext\CNext\RadeonSoftware.exe"),
        # The successor of Intel Graphics Command Center, which no longer tells about new drivers.
        INTEL: ("Intel Graphics Software", r"Intel\Intel Graphics Software\IntelGraphicsSoftware.exe"),
    }[vendor]
    path = Path(os.environ.get("ProgramFiles") or r"C:\Program Files") / exe
    return VendorApp(name, str(path)) if path.is_file() else None
