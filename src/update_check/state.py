"""Checklist steps, the weekly reminder cycle and persistence."""

import json
import os
from collections.abc import Callable
from dataclasses import asdict, dataclass, field
from datetime import UTC, date, datetime, timedelta
from pathlib import Path
from typing import Any

WEEKDAYS = ("Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday")
APPEARANCES = ("System", "Light", "Dark")
# The step that lists the PC's graphics cards with their driver status, instead of a fixed description.
GRAPHICS_STEP = "graphics"
# The step that says how much room is left on the system drive.
STORAGE_STEP = "storage"
# The default steps from before checklists recorded which default steps they had been offered.
_ORIGINAL_DEFAULT_STEPS = ("windows-update", "drivers", "store", "winget", "defender", "backup")


def local_now() -> datetime:
    """The current local time, timezone-aware."""
    return datetime.now(UTC).astimezone()


def _new_id() -> str:
    # uuid is imported here rather than at the top: only a step added by hand needs an id, while every
    # background check would pay for the import.
    import uuid

    return uuid.uuid4().hex[:8]


@dataclass
class Step:
    title: str
    detail: str = ""
    # URL, ms-settings: page, file path or shell command opened by the step's "Open" button.
    action: str = ""
    id: str = field(default_factory=_new_id)


def default_steps() -> list[Step]:
    return [
        Step(
            "Windows Update",
            "Install pending updates, then restart if asked.",
            "ms-settings:windowsupdate",
            id="windows-update",
        ),
        Step(
            "Optional driver updates",
            "Review optional updates for drivers and firmware.",
            "ms-settings:windowsupdate-optionalupdates",
            id="drivers",
        ),
        Step("Graphics drivers", id=GRAPHICS_STEP),
        Step(
            "Microsoft Store apps",
            "Open the Library and click “Get updates”.",
            "ms-windows-store://downloadsandupdates",
            id="store",
        ),
        Step(
            "Desktop apps",
            "Upgrade everything winget manages.",
            "winget upgrade --all",
            id="winget",
        ),
        Step(
            "Microsoft Defender",
            "Check for new protection updates.",
            "windowsdefender://threat",
            id="defender",
        ),
        Step(
            "Free up space",
            "Delete temporary files, or turn on Storage Sense to do it automatically.",
            "ms-settings:storagesense",
            id=STORAGE_STEP,
        ),
        Step(
            "Backups",
            "Make sure the latest backup finished without errors.",
            id="backup",
        ),
    ]


@dataclass
class Settings:
    weekday: int = 0
    hour: int = 10
    appearance: str = "System"
    # Start with Windows so reminders can appear. None until the .exe has asked, on its first start; from then on
    # the .exe applies the answer at every start.
    autostart: bool | None = None


@dataclass
class State:
    steps: list[Step] = field(default_factory=default_steps)
    settings: Settings = field(default_factory=Settings)
    # The day the weekly cycle that `done` belongs to started.
    cycle_start: date | None = None
    done: set[str] = field(default_factory=set)
    last_completed: datetime | None = None
    last_reminded: date | None = None
    # Ids of the default steps this checklist has been offered, so a default step added in an update shows up
    # once in existing checklists, and stays away once removed.
    offered_steps: set[str] = field(default_factory=lambda: {step.id for step in default_steps()})
    # The day each step was last ticked, so a step nobody has touched for months can say so.
    last_done: dict[str, date] = field(default_factory=dict)
    # The start of every weekly cycle that was finished, oldest first, for the streak.
    completed_weeks: list[date] = field(default_factory=list)

    @property
    def is_complete(self) -> bool:
        return bool(self.steps) and all(step.id in self.done for step in self.steps)

    def current_cycle_start(self, now: datetime) -> date:
        """The most recent reminder day whose reminder hour has passed.

        Works on local wall-clock date and hour, so a daylight saving change can't shift the cycle.
        """
        days_back = (now.weekday() - self.settings.weekday) % 7
        if days_back == 0 and now.hour < self.settings.hour:
            days_back = 7
        return now.date() - timedelta(days=days_back)

    def next_reminder(self, now: datetime) -> date:
        return self.current_cycle_start(now) + timedelta(days=7)

    def roll_over(self, now: datetime) -> bool:
        """Start a fresh checklist once a newer weekly cycle has begun. Returns True if it did."""
        start = self.current_cycle_start(now)
        # Only move forward, so moving the reminder to an earlier day keeps this week's progress.
        if self.cycle_start is not None and start <= self.cycle_start:
            return False
        self.cycle_start = start
        self.done.clear()
        return True

    def set_done(self, step_id: str, done: bool, now: datetime) -> None:
        if done:
            self.done.add(step_id)
            self.last_done[step_id] = now.date()
        else:
            self.done.discard(step_id)
        if self.is_complete:
            self.last_completed = now
            if self.cycle_start is not None and self.cycle_start not in self.completed_weeks:
                self.completed_weeks.append(self.cycle_start)
                # Five years is plenty to draw a streak from, and keeps the state file small.
                del self.completed_weeks[:-260]
        elif self.cycle_start is not None and self.completed_weeks[-1:] == [self.cycle_start]:
            # Un-ticking a step takes this week back off the streak.
            self.completed_weeks.pop()

    def streak(self, now: datetime) -> int:
        """How many weekly cycles in a row were finished, counting back from this one. A cycle counts when
        a completion falls anywhere in its seven days, so moving the reminder day keeps the streak."""
        def finished(start: date) -> bool:
            return any(start <= week < start + timedelta(days=7) for week in self.completed_weeks)

        start = self.current_cycle_start(now)
        if not finished(start):
            # This week isn't done yet, which doesn't break a streak until the week is over.
            start -= timedelta(days=7)
        streak = 0
        while finished(start):
            streak += 1
            start -= timedelta(days=7)
        return streak

    def should_remind(self, now: datetime) -> bool:
        """Whether a background check should open the window. Call `roll_over` first."""
        return bool(self.steps) and not self.is_complete and self.last_reminded != now.date()

    def add_new_default_steps(self) -> None:
        """Add the default steps this checklist hasn't been offered yet, each after the last of the earlier
        default steps it has, or at the end."""
        defaults = default_steps()
        for index, step in enumerate(defaults):
            if step.id in self.offered_steps:
                continue
            self.offered_steps.add(step.id)
            ids = [existing.id for existing in self.steps]
            if step.id not in ids:
                earlier = [ids.index(default.id) for default in defaults[:index] if default.id in ids]
                self.steps.insert(max(earlier) + 1 if earlier else len(self.steps), step)

    def to_json(self) -> dict[str, Any]:
        return {
            "steps": [asdict(step) for step in self.steps],
            "settings": asdict(self.settings),
            "cycle_start": _format(self.cycle_start),
            "done": sorted(self.done),
            "last_completed": _format(self.last_completed),
            "last_reminded": _format(self.last_reminded),
            "offered_steps": sorted(self.offered_steps),
            "last_done": {step: day.isoformat() for step, day in sorted(self.last_done.items())},
            "completed_weeks": [week.isoformat() for week in self.completed_weeks],
        }

    @classmethod
    def from_json(cls, data: dict[str, Any]) -> State:
        state = cls(
            steps=[Step(**step) for step in data["steps"]],
            settings=Settings(**data["settings"]),
            cycle_start=_parse(data.get("cycle_start"), _parse_date),
            done=set(data.get("done", [])),
            last_completed=_parse(data.get("last_completed"), datetime.fromisoformat),
            last_reminded=_parse(data.get("last_reminded"), _parse_date),
            offered_steps=set(data.get("offered_steps", _ORIGINAL_DEFAULT_STEPS)),
            last_done={step: _parse_date(day) for step, day in data.get("last_done", {}).items()},
            completed_weeks=[_parse_date(week) for week in data.get("completed_weeks", [])],
        )
        state.add_new_default_steps()
        return state


def _format(value: date | None) -> str | None:
    return value.isoformat() if value else None


def _parse[T](value: str | None, parser: Callable[[str], T]) -> T | None:
    return parser(value) if value else None


def _parse_date(value: str) -> date:
    # Also accepts a full timestamp, which earlier versions stored for the cycle start.
    return datetime.fromisoformat(value).date()


def data_file() -> Path:
    if override := os.environ.get("UPDATE_CHECK_DATA"):
        return Path(override)
    return Path(os.environ.get("APPDATA") or Path.home()) / "UpdateCheck" / "state.json"


def load(path: Path) -> State:
    try:
        return State.from_json(json.loads(path.read_text(encoding="utf-8")))
    except FileNotFoundError:
        return State()
    except (ValueError, TypeError, KeyError):
        # Keep the unreadable file for inspection instead of silently overwriting it.
        path.replace(path.with_name(path.name + ".bak"))
        return State()


def save(state: State, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + ".tmp")
    temp.write_text(json.dumps(state.to_json(), indent=2, ensure_ascii=False), encoding="utf-8")
    os.replace(temp, path)
