"""How much is actually waiting: pending Windows updates and winget package upgrades.

Neither tool offers a machine-readable count. winget prints a table in whatever language Windows is set
to, and Windows Update answers only through COM, so both are read defensively: anything that doesn't look
like an answer is reported as unknown rather than guessed at. Both cost seconds and a network round trip,
so they run in background threads and the checklist shows whatever has come back.
"""

import logging
import queue
import re
import subprocess
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass

log = logging.getLogger(__name__)

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


def _run(command: list[str], timeout: int) -> str | None:
    """The command's output, or None if it couldn't be run. CREATE_NO_WINDOW keeps a console from
    flashing up over the checklist."""
    try:
        result = subprocess.run(
            command, check=False, capture_output=True, timeout=timeout,
            encoding="utf-8", errors="replace", creationflags=subprocess.CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout


def winget_upgrades() -> int | None:
    """How many packages winget can upgrade, or None if its answer couldn't be read."""
    output = _run(["winget", "upgrade", "--disable-interactivity"], _WINGET_TIMEOUT)
    return count_winget_table(output) if output is not None else None


def count_winget_table(output: str) -> int | None:
    """Count the rows of winget's upgrade table.

    The table is found by shape rather than by its words, which are translated: a rule of dashes, then one
    row per package, then a summary sentence. A row is padded out into columns, so somewhere along it two
    spaces sit together; the sentence underneath is ordinary prose and never has that. A column whose text
    exactly fills its width leaves only a single space, so one such gap is enough to call it a row.
    """
    lines = output.splitlines()
    for index, line in enumerate(lines):
        stripped = line.strip()
        if len(stripped) > 10 and set(stripped) == {"-"}:
            count = 0
            for row in lines[index + 1:]:
                if not re.search(r"\S {2,}\S", row) or len(row.strip()) * 3 < len(stripped):
                    break
                count += 1
            return count if count < _IMPLAUSIBLE else None
    # No rule at all means winget said something else, such as that everything is up to date.
    return 0 if lines else None


def windows_updates() -> int | None:
    """How many updates Windows Update is holding. Only COM can be asked, so PowerShell asks for it."""
    script = (
        "$ErrorActionPreference='Stop';"
        "$searcher = (New-Object -ComObject Microsoft.Update.Session).CreateUpdateSearcher();"
        "$searcher.Search(\"IsInstalled=0 and Type='Software' and IsHidden=0\").Updates.Count"
    )
    output = _run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", script], _WINDOWS_UPDATE_TIMEOUT
    )
    if output is None:
        return None
    text = output.strip()
    return int(text) if text.isdigit() and int(text) < _IMPLAUSIBLE else None


# The steps whose counts can be looked up, by step id. A checklist without them asks nothing.
LOOKUPS: dict[str, Callable[[], int | None]] = {
    "winget": winget_upgrades,
    "windows-update": windows_updates,
}

# What to call what is waiting, per step.
_NOUNS = {"winget": ("upgrade", "upgrades"), "windows-update": ("update", "updates")}


def describe(step_id: str, count: int) -> str:
    """'12 upgrades waiting', in the words that suit the step."""
    one, many = _NOUNS.get(step_id, ("item", "items"))
    return f"{count} {one if count == 1 else many} waiting"


@dataclass(frozen=True)
class Lookup:
    count: int | None = None
    failed: bool = False


class PendingChecks:
    """Counts of what each step has waiting, looked up in background threads.

    `refresh()` starts the lookups for the steps a checklist actually has; `collect()`, called from the
    window's thread, takes in the finished ones.
    """

    # Seconds before a failed lookup is tried again, e.g. once the PC is back online.
    RETRY_AFTER = 60

    def __init__(self, lookups: dict[str, Callable[[], int | None]] = LOOKUPS):
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
        if result.failed or result.count is None:
            return CHECK_FAILED, 0
        return (PENDING if result.count else NOTHING_PENDING), result.count

    def _run(self, step_id: str) -> None:
        try:
            result = Lookup(self._lookups[step_id]())
        except Exception:
            # These are other people's command line tools: whatever goes wrong, the count is unknown.
            log.warning("Couldn't count what %s has waiting", step_id, exc_info=True)
            result = Lookup(failed=True)
        self._finished.put((step_id, result))
