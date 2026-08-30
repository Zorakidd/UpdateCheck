"""Checklist steps, the weekly reminder cycle and persistence."""

import json
import os
import uuid
from dataclasses import asdict, dataclass, field
from datetime import UTC, date, datetime, timedelta
from pathlib import Path

WEEKDAYS = ("Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday")
APPEARANCES = ("System", "Light", "Dark")


def local_now() -> datetime:
    """The current local time, timezone-aware."""
    return datetime.now(UTC).astimezone()


@dataclass
class Step:
    title: str
    detail: str = ""
    # URL, ms-settings: page, file path or shell command opened by the step's "Open" button.
    action: str = ""
    id: str = field(default_factory=lambda: uuid.uuid4().hex[:8])


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
    # Start with Windows so reminders can appear. Applied automatically when running as the .exe.
    autostart: bool = True


@dataclass
class State:
    steps: list[Step] = field(default_factory=default_steps)
    settings: Settings = field(default_factory=Settings)
    # The day the weekly cycle that `done` belongs to started.
    cycle_start: date | None = None
    done: set[str] = field(default_factory=set)
    last_completed: datetime | None = None
    last_reminded: date | None = None

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
        else:
            self.done.discard(step_id)
        if self.is_complete:
            self.last_completed = now

    def should_remind(self, now: datetime) -> bool:
        """Whether a background check should open the window. Call `roll_over` first."""
        return bool(self.steps) and not self.is_complete and self.last_reminded != now.date()

    def to_json(self) -> dict:
        return {
            "steps": [asdict(step) for step in self.steps],
            "settings": asdict(self.settings),
            "cycle_start": _format(self.cycle_start),
            "done": sorted(self.done),
            "last_completed": _format(self.last_completed),
            "last_reminded": _format(self.last_reminded),
        }

    @classmethod
    def from_json(cls, data: dict) -> State:
        return cls(
            steps=[Step(**step) for step in data["steps"]],
            settings=Settings(**data["settings"]),
            cycle_start=_parse(data.get("cycle_start"), _parse_date),
            done=set(data.get("done", [])),
            last_completed=_parse(data.get("last_completed"), datetime.fromisoformat),
            last_reminded=_parse(data.get("last_reminded"), _parse_date),
        )


def _format(value: date | None) -> str | None:
    return value.isoformat() if value else None


def _parse(value, parser):
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
