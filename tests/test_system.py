import contextlib

import pytest

from update_check import system


def test_repair_runs_dism_then_sfc_as_administrator(monkeypatch):
    calls = []
    monkeypatch.setattr(system.os, "startfile", lambda *args: calls.append(args))

    system.repair_windows()

    [(program, operation, arguments)] = calls
    assert (program, operation) == ("cmd.exe", "runas")
    assert arguments.startswith("/k ")
    assert arguments.index("DISM /Online /Cleanup-Image /RestoreHealth") < arguments.index("sfc /scannow")


def test_declining_the_administrator_prompt_is_not_an_error(monkeypatch):
    def declined(*args):
        raise OSError(None, "The operation was canceled by the user.", None, 1223)

    monkeypatch.setattr(system.os, "startfile", declined)

    system.repair_windows()


def test_other_failures_to_start_the_repair_are_raised(monkeypatch):
    def missing(*args):
        raise OSError(None, "The system cannot find the file specified.", None, 2)

    monkeypatch.setattr(system.os, "startfile", missing)

    with pytest.raises(OSError):
        system.repair_windows()


@pytest.mark.parametrize(
    ("count", "text"),
    [
        (32 * 1024**3, "32 GB"),
        (int(15.9 * 1024**3), "15.9 GB"),
        (8 * 1024**3, "8 GB"),
        (954 * 1024**3, "954 GB"),
        (1862 * 1024**3, "1.82 TB"),
    ],
)
def test_sizes_are_given_to_three_figures(count, text):
    assert system.size(count) == text


def test_the_system_drive_is_read_from_windows():
    space = system.system_drive_space()
    assert space is not None
    drive, free, total = space
    assert drive.endswith(":")
    assert 0 < free <= total


def test_without_a_policy_the_app_updates_itself(monkeypatch):
    def no_key(*args, **kwargs):
        raise FileNotFoundError

    monkeypatch.setattr(system.winreg, "OpenKey", no_key)
    assert system.policy("DisableSelfUpdate") is None
    assert system.self_update_allowed()


def test_an_administrator_can_turn_self_update_off(monkeypatch):
    monkeypatch.setattr(system, "policy", lambda name: 1 if name == "DisableSelfUpdate" else None)
    assert not system.self_update_allowed()


def test_each_user_gets_their_own_daily_task(monkeypatch):
    monkeypatch.setenv("USERNAME", "Zoë Müller")
    assert system.task_name() == "UpdateCheck (Zoë Müller)"


def test_turning_the_reminder_on_replaces_the_shared_task_with_the_users_own(monkeypatch):
    monkeypatch.setenv("USERNAME", "sam")
    calls = []
    monkeypatch.setattr(system, "_schtasks", lambda *args, check=True: calls.append((args, check)))
    monkeypatch.setattr(system.winreg, "CreateKey", lambda *args: contextlib.nullcontext())
    monkeypatch.setattr(system.winreg, "SetValueEx", lambda *args: None)

    system.enable_autostart(10)

    (create, create_checked), (delete, delete_checked) = calls
    assert create[:2] == ("/Create", "/F") and create[create.index("/TN") + 1] == "UpdateCheck (sam)"
    assert create_checked
    # Another user's shared task can't be deleted, which is fine.
    assert delete == ("/Delete", "/F", "/TN", "UpdateCheck") and not delete_checked
