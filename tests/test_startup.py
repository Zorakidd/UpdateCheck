import subprocess
import sys
import types
from datetime import timedelta

import pytest

from update_check import startup, system
from update_check.startup import _UPDATE_INTERVAL, _update_due
from update_check.state import Settings, State, local_now, save


@pytest.fixture
def start_exe(monkeypatch, tmp_path):
    """Start up as the .exe does, without asking GitHub or touching Windows. Returns what start-up registered
    and, for each time it opened the window, whether as a reminder."""
    path = tmp_path / "state.json"
    monkeypatch.setenv("UPDATE_CHECK_DATA", str(path))
    monkeypatch.setattr(system, "FROZEN", True)
    monkeypatch.setattr(system, "claim_single_instance", lambda: 1)
    monkeypatch.setattr(startup, "_update_due", lambda *args: False)
    monkeypatch.setattr(startup, "_start_logging", lambda path: None)
    monkeypatch.setattr(system, "user_is_busy", lambda: False)
    registered: list[bool] = []
    opened: list[bool] = []
    monkeypatch.setattr(system, "sync_autostart", lambda enabled, hour: registered.append(enabled))
    window = types.SimpleNamespace(open_window=lambda data, path, reminder: opened.append(reminder))
    monkeypatch.setitem(sys.modules, "update_check.app", window)

    def start(saved: State | None = None, check_only: bool = False) -> tuple[list[bool], list[bool]]:
        if saved is not None:
            save(saved, path)
        startup.run(check_only=check_only)
        return registered, opened

    return start


def test_background_check_does_not_import_the_window_toolkit():
    # Importing customtkinter was over half of a background check's run time, and most checks never open the window.
    code = "import sys, update_check.startup; sys.exit('customtkinter' in sys.modules)"
    assert subprocess.run([sys.executable, "-c", code], check=False).returncode == 0


def test_background_check_does_not_import_the_self_updater():
    # Only the .exe updates itself, and the updater brings urllib with it, which brings hashlib and shutil.
    code = (
        "import sys, update_check.startup;"
        " sys.exit(bool({'update_check.updater', 'urllib.request'} & sys.modules.keys()))"
    )
    assert subprocess.run([sys.executable, "-c", code], check=False).returncode == 0


def test_background_check_does_not_import_argparse_logging_or_uuid():
    # A quarter of a background check's imports: argparse parsed one flag, logging drags in traceback and
    # dataclasses' inspect chain, and uuid only ever names a step added by hand.
    code = (
        "import sys, update_check, update_check.startup;"
        " sys.exit(bool({'argparse', 'logging', 'uuid'} & sys.modules.keys()))"
    )
    assert subprocess.run([sys.executable, "-c", code], check=False).returncode == 0


def test_opening_the_window_does_not_import_the_driver_lookup_s_parsers():
    # A fifth of the window's import time. Only the NVIDIA lookup needs them, and it runs in a background
    # thread that is already waiting on the network.
    code = (
        "import sys, update_check.app;"
        " sys.exit(bool({'urllib.request', 'xml.etree.ElementTree'} & sys.modules.keys()))"
    )
    assert subprocess.run([sys.executable, "-c", code], check=False).returncode == 0


def test_opening_the_window_does_not_read_the_pc_yet():
    # Only the PC info tab needs it, and most runs never open that tab.
    code = "import sys, update_check.app; sys.exit('update_check.pcinfo' in sys.modules)"
    assert subprocess.run([sys.executable, "-c", code], check=False).returncode == 0


def test_the_first_start_sets_nothing_up_before_asking(start_exe):
    registered, opened = start_exe()
    assert registered == []
    # The window asks; opened by hand, not as a reminder.
    assert opened == [False]


def test_later_starts_apply_the_answer(start_exe):
    registered, _ = start_exe(State(settings=Settings(autostart=False)))
    assert registered == [False]


def test_where_an_administrator_rolls_out_versions_github_isnt_asked(start_exe, monkeypatch):
    asked = []
    monkeypatch.setattr(startup, "_update_due", lambda *args: asked.append(1) or False)
    monkeypatch.setattr(system, "self_update_allowed", lambda: False)
    start_exe()
    assert asked == []


def test_an_outgrown_log_is_moved_aside(tmp_path, monkeypatch):
    log = tmp_path / "update-check.log"
    log.write_text("old\n" * 10, encoding="utf-8")
    startup.rotate(log)
    assert log.exists()

    monkeypatch.setattr(startup, "_LOG_LIMIT", 5)
    startup.rotate(log)
    assert not log.exists()
    assert (tmp_path / "update-check.log.1").read_text(encoding="utf-8").startswith("old")


def _crash(check_only: bool) -> None:
    raise RuntimeError("boom")


def test_an_unexpected_error_in_the_exe_is_logged_not_shown_as_a_traceback(monkeypatch, caplog, tmp_path):
    monkeypatch.setenv("UPDATE_CHECK_DATA", str(tmp_path / "state.json"))
    monkeypatch.setattr(startup, "_run", _crash)
    monkeypatch.setattr(startup, "_start_logging", lambda path: None)
    monkeypatch.setattr(system, "FROZEN", True)

    # A background check: nobody is waiting for a window, so no message either.
    with pytest.raises(SystemExit) as stopped:
        startup.run(check_only=True)

    assert stopped.value.code == 1
    assert "stopped on an unexpected error" in caplog.text
    assert "RuntimeError: boom" in caplog.text


def test_run_from_source_an_unexpected_error_still_shows_its_traceback(monkeypatch, tmp_path):
    monkeypatch.setenv("UPDATE_CHECK_DATA", str(tmp_path / "state.json"))
    monkeypatch.setattr(startup, "_run", _crash)
    monkeypatch.setattr(startup, "_start_logging", lambda path: None)
    monkeypatch.setattr(system, "FROZEN", False)

    with pytest.raises(RuntimeError, match="boom"):
        startup.run(check_only=False)


def test_a_reminder_waits_while_a_game_or_presentation_is_full_screen(start_exe, monkeypatch):
    monkeypatch.setattr(system, "user_is_busy", lambda: True)
    _, opened = start_exe(State(settings=Settings(autostart=True)), check_only=True)
    assert opened == []

    # Still due, so the next check reminds.
    monkeypatch.setattr(system, "user_is_busy", lambda: False)
    _, opened = start_exe(check_only=True)
    assert opened == [True]


def test_the_background_check_stays_lean():
    # A ceiling rather than a list of names, so an import nobody thought to guard is caught too.
    code = "import sys, update_check, update_check.startup; print(len(sys.modules))"
    loaded = int(subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=True).stdout)
    assert loaded < 130, f"the background check now imports {loaded} modules"


def test_the_first_run_asks_about_a_new_release(tmp_path):
    assert _update_due(tmp_path, local_now()) is True


def test_the_next_run_does_not_ask_again(tmp_path):
    now = local_now()
    _update_due(tmp_path, now)
    assert _update_due(tmp_path, now + _UPDATE_INTERVAL - timedelta(seconds=1)) is False


def test_it_asks_again_once_the_interval_has_passed(tmp_path):
    now = local_now()
    _update_due(tmp_path, now)
    assert _update_due(tmp_path, now + _UPDATE_INTERVAL) is True


def test_an_unreadable_marker_does_not_stop_the_update_check(tmp_path):
    (tmp_path / "last-checked.txt").write_text("not a timestamp", encoding="utf-8")
    assert _update_due(tmp_path, local_now()) is True
