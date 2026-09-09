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
