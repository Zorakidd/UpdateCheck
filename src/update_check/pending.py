"""How much is actually waiting: pending Windows updates, driver updates and winget package upgrades.

Neither tool offers a machine-readable count. winget prints a table in whatever language Windows is set
to, and Windows Update answers only through COM, so both are read defensively: anything that doesn't look
like an answer is reported as unknown rather than guessed at. Both cost seconds and a network round trip,
so they run in background threads and the checklist shows whatever has come back.
"""

import logging
import queue
import subprocess
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import date

log = logging.getLogger(__name__)

# The steps whose counts are looked up.
WINDOWS_UPDATE, DRIVERS, WINGET = "windows-update", "drivers", "winget"

# What is known about a step's count.
CHECKING = "checking"
NOTHING_PENDING = "nothing pending"
PENDING = "pending"
CHECK_FAILED = "check failed"

# Windows Update's COM search regularly takes 15 seconds; winget has to reach its sources.
_WINDOWS_UPDATE_TIMEOUT = 120
_WINGET_TIMEOUT = 90
# Counts this large mean the output was misread, not that the PC is that far behind.
_IMPLAUSIBLE = 1000
# How much shorter than the table a row may end: by as much as its source's name, such as "winget", is shorter
# than the last column, which is as wide as the longest source name or the column's translated heading.
_SOURCE_SLACK = 12

# One search for both Windows Update steps: type 1 is software, Windows itself included, and type 2 drivers and
# firmware. The day updates were last installed comes along, which tells how current Windows is. The answer is
# a single line such as "3 1 2026-11-03", with "-" for a day Windows can't tell.
_SEARCH_SCRIPT = (
    "$ErrorActionPreference='Stop';"
    "$updates = (New-Object -ComObject Microsoft.Update.Session).CreateUpdateSearcher()"
    ".Search('IsInstalled=0 and IsHidden=0').Updates;"
    "$software = @($updates | Where-Object { $_.Type -eq 1 }).Count;"
    "$drivers = @($updates | Where-Object { $_.Type -eq 2 }).Count;"
    "$installed = (New-Object -ComObject Microsoft.Update.AutoUpdate).Results.LastInstallationSuccessDate;"
    "$day = if ($installed -and $installed.Year -gt 2000) { $installed.ToString('yyyy-MM-dd') } else { '-' };"
    "($software, $drivers, $day) -join ' '"
)


@dataclass(frozen=True)
class Found:
    """How much a lookup found waiting and, for Windows Update, the day it last installed updates."""

    count: int
    last_installed: date | None = None


def _run(command: list[str], timeout: int) -> subprocess.CompletedProcess[str] | None:
    """The finished command, or None if it couldn't be run. CREATE_NO_WINDOW keeps a console from
    flashing up over the checklist."""
    try:
        return subprocess.run(
            command, check=False, capture_output=True, timeout=timeout,
            encoding="utf-8", errors="replace", creationflags=subprocess.CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError):
        return None


def winget_upgrades() -> int | None:
    """How many packages winget can upgrade, or None if its answer couldn't be read."""
    result = _run(["winget", "upgrade", "--disable-interactivity"], _WINGET_TIMEOUT)
    return count_winget_table(result.stdout, result.returncode) if result is not None else None


def count_winget_table(output: str, exit_code: int = 0) -> int | None:
    """Count the rows of winget's upgrade table.

    The table is found by shape rather than by its words, which are translated: a rule of dashes as wide as
    the table, then one row per package, then a summary sentence. winget pads every column but the last, so
    a package's row runs as wide as the rule, or nearly: only the source's name in the last column can fall
    short of it. The sentence underneath is shorter, or ends in a full stop rather than a source's name.

    Spaces don't tell them apart: where a package has the widest entry in every column, its row has nothing
    but single spaces, just like a sentence.

    Without a table, only the exit code tells "nothing to upgrade" from an error: winget explains both in a
    single translated sentence.
    """
    lines = output.splitlines()
    for index, line in enumerate(lines):
        stripped = line.strip()
        if len(stripped) > 10 and set(stripped) == {"-"}:
            width = _width(line.rstrip())
            count = 0
            for row in lines[index + 1:]:
                text = row.rstrip()
                if not (width - _SOURCE_SLACK <= _width(text) <= width + 1 and text[-1:].isalnum()):
                    break
                count += 1
            return count if count < _IMPLAUSIBLE else None
    # No rule at all means winget said something else: that everything is up to date if it exited cleanly,
    # or why it failed.
    return 0 if lines and exit_code == 0 else None


def _width(text: str) -> int:
    """How many columns a console gives the text: two for each wide character, as in Chinese or Japanese."""
    # Imported here, in the background thread counting winget's table, rather than when the window opens.
    import unicodedata

    return sum(2 if unicodedata.east_asian_width(char) in "WF" else 1 for char in text)


@dataclass(frozen=True)
class WindowsUpdateSearch:
    software: int
    drivers: int
    last_installed: date | None


def search_windows_update() -> WindowsUpdateSearch | None:
    """What Windows Update has waiting. Only COM can be asked, so PowerShell asks for it."""
    result = _run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", _SEARCH_SCRIPT], _WINDOWS_UPDATE_TIMEOUT
    )
    if result is None or result.returncode:
        return None
    return parse_search(result.stdout)


def parse_search(output: str) -> WindowsUpdateSearch | None:
    """Read the search's answer, such as "3 1 2026-11-03", or None if it isn't one."""
    parts = output.split()
    if len(parts) != 3 or not (parts[0].isdigit() and parts[1].isdigit()):
        return None
    software, drivers = int(parts[0]), int(parts[1])
    if max(software, drivers) >= _IMPLAUSIBLE:
        return None
    try:
        last_installed = date.fromisoformat(parts[2])
    except ValueError:
        last_installed = None
    return WindowsUpdateSearch(software, drivers, last_installed)


class SharedSearch:
    """Windows Update searched once for every lookup that asks within a minute. A search takes a while, and a
    second one started alongside would only wait for the first."""

    FRESH_FOR = 60

    def __init__(self, search: Callable[[], WindowsUpdateSearch | None] = search_windows_update):
        self._search = search
        self._lock = threading.Lock()
        self._result: WindowsUpdateSearch | None = None
        self._searched_at: float | None = None

    def __call__(self) -> WindowsUpdateSearch | None:
        with self._lock:
            if self._searched_at is None or time.monotonic() - self._searched_at >= self.FRESH_FOR:
                self._result = self._search()
                self._searched_at = time.monotonic()
            return self._result


_windows_update = SharedSearch()


def windows_updates() -> Found | None:
    """The updates Windows Update holds for Windows and other software."""
    search = _windows_update()
    return Found(search.software, search.last_installed) if search else None


def driver_updates() -> Found | None:
    """The driver and firmware updates Windows Update offers, which on many brand PCs include the BIOS."""
    search = _windows_update()
    return Found(search.drivers) if search else None


# The steps whose counts can be looked up, by step id. A checklist without them asks nothing.
LOOKUPS: dict[str, Callable[[], int | Found | None]] = {
    WINGET: winget_upgrades,
    WINDOWS_UPDATE: windows_updates,
    DRIVERS: driver_updates,
}

# What to call what is waiting, per step.
_NOUNS = {
    WINGET: ("upgrade", "upgrades"),
    WINDOWS_UPDATE: ("update", "updates"),
    DRIVERS: ("driver update", "driver updates"),
}


def describe(step_id: str, count: int) -> str:
    """'12 upgrades waiting', in the words that suit the step."""
    one, many = _NOUNS.get(step_id, ("item", "items"))
    return f"{count} {one if count == 1 else many} waiting"


@dataclass(frozen=True)
class Lookup:
    found: Found | None = None
    failed: bool = False


class PendingChecks:
    """Counts of what each step has waiting, looked up in background threads.

    `refresh()` starts the lookups for the steps a checklist actually has; `collect()`, called from the
    window's thread, takes in the finished ones.
    """

    # Seconds before a failed lookup is tried again, e.g. once the PC is back online.
    RETRY_AFTER = 60

    def __init__(self, lookups: dict[str, Callable[[], int | Found | None]] = LOOKUPS):
        self._lookups = lookups
        self._results: dict[str, Lookup] = {}
        self._failed_at: dict[str, float] = {}
        self._running: set[str] = set()
        self._finished: queue.SimpleQueue[tuple[str, Lookup]] = queue.SimpleQueue()

    @property
    def busy(self) -> bool:
        return bool(self._running)

    def refresh(self, step_ids: set[str], recheck: bool = False) -> bool:
        """Look up the counts for the steps in `step_ids` that haven't been counted, whose lookup failed a
        while ago, or all of them with `recheck`. Returns True if anything shown changed."""
        changed = False
        for step_id in self._lookups.keys() & step_ids:
            if step_id in self._running:
                continue
            result = self._results.get(step_id)
            retry = (
                result is not None
                and result.failed
                and time.monotonic() - self._failed_at[step_id] >= self.RETRY_AFTER
            )
            if recheck or result is None or retry:
                self._results.pop(step_id, None)
                self._running.add(step_id)
                threading.Thread(target=self._run, args=(step_id,), daemon=True).start()
                changed = True
        return changed

    def collect(self) -> bool:
        """Take in finished lookups. Returns True if there were any."""
        collected = False
        while not self._finished.empty():
            step_id, result = self._finished.get()
            self._running.discard(step_id)
            self._results[step_id] = result
            if result.failed:
                self._failed_at[step_id] = time.monotonic()
            collected = True
        return collected

    def status(self, step_id: str) -> tuple[str, int]:
        """The step's status and, when there is one, the count behind it."""
        if step_id not in self._lookups:
            return CHECK_FAILED, 0
        result = self._results.get(step_id)
        if result is None:
            return CHECKING, 0
        if result.failed or result.found is None:
            return CHECK_FAILED, 0
        count = result.found.count
        return (PENDING if count else NOTHING_PENDING), count

    def last_installed(self, step_id: str) -> date | None:
        """The day Windows last installed updates, once the step's lookup has found out."""
        result = self._results.get(step_id)
        return result.found.last_installed if result is not None and result.found is not None else None

    def _run(self, step_id: str) -> None:
        try:
            value = self._lookups[step_id]()
            result = Lookup(Found(value) if isinstance(value, int) else value)
        except Exception:
            # These are other people's command line tools: whatever goes wrong, the count is unknown.
            log.warning("Couldn't count what %s has waiting", step_id, exc_info=True)
            result = Lookup(failed=True)
        self._finished.put((step_id, result))
