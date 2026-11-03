"""Start-up before the window opens: single instance, self-update, autostart and the reminder check."""

import ctypes
from datetime import datetime, timedelta
from pathlib import Path

from update_check import system
from update_check.state import data_file, load, local_now, save

# How long a background check waits before asking GitHub about a new release again. Releases are rare, and
# skipping the question also skips importing the updater, which brings ssl, http, email, zipfile and shutil.
_UPDATE_INTERVAL = timedelta(days=1)
# The log is kept to about this size, with the one before it beside it as update-check.log.1.
_LOG_LIMIT = 1024 * 1024
_MB_ICONERROR = 0x10


def _start_logging(path: Path) -> None:
    """Open the log file. Called only from the paths that can log, because importing logging costs about
    a third of a background check's start-up and a check with nothing to report never writes a line.
    Safe to call twice: basicConfig does nothing once the root logger has a handler."""
    import logging

    log_file = path.with_name("update-check.log")
    rotate(log_file)
    logging.basicConfig(
        filename=log_file,
        encoding="utf-8",
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        level=logging.INFO,
    )


def rotate(log_file: Path) -> None:
    """Move a log that has outgrown _LOG_LIMIT aside, replacing the one moved aside before it."""
    try:
        if log_file.stat().st_size > _LOG_LIMIT:
            log_file.replace(log_file.with_name(log_file.name + ".1"))
    except OSError:
        # Missing, or open in the window's process: the next start tries again.
        pass


def _update_due(data_dir: Path, now: datetime) -> bool:
    """Whether this run should ask GitHub for a newer release. The time is recorded before the question is
    asked, so a failed check waits for the next interval instead of retrying at every sign-in."""
    marker = data_dir / "last-checked.txt"
    try:
        if now - datetime.fromisoformat(marker.read_text(encoding="utf-8").strip()) < _UPDATE_INTERVAL:
            return False
    except (OSError, ValueError):
        pass
    try:
        marker.write_text(now.isoformat(), encoding="utf-8")
    except OSError:
        pass
    return True


def run(check_only: bool) -> None:
    """Open the checklist. With `check_only`, open it only when this week's check still needs a reminder."""
    try:
        _run(check_only)
    except Exception:
        # The .exe has no console for the traceback: the log keeps it for whoever supports the PC.
        _start_logging(data_file())
        import logging

        logging.getLogger(__name__).exception("Update Check stopped on an unexpected error")
        if not system.FROZEN:
            raise
        if not check_only:
            # Opened by hand, so someone is waiting for the window. A background check stays quiet.
            log_file = data_file().with_name("update-check.log")
            message = f"Update Check couldn't start because of an unexpected error. The details are in\n{log_file}"
            ctypes.windll.user32.MessageBoxW(None, message, system.TITLE, _MB_ICONERROR)
        raise SystemExit(1) from None


def _run(check_only: bool) -> None:
    path = data_file()
    now = local_now()
    lock = system.claim_single_instance()
    if lock is None:
        # The open window saves its own state, so only read it here.
        data = load(path)
        data.roll_over(now)
        if not check_only or (data.should_remind(now) and not system.user_is_busy()):
            system.focus_window(system.TITLE)
        return

    path.parent.mkdir(parents=True, exist_ok=True)
    # Holding the lock means no other copy is running from the .exe that the update replaces. Only the .exe
    # updates itself, and importing the updater costs more than the rest of a background check put together,
    # so both are left out of runs from source and of the path that just focuses an already open window.
    # Once a day is often enough to ask, which keeps that cost and an HTTPS round trip out of most runs. Where
    # an administrator rolls out new versions themselves, it isn't asked at all.
    if system.FROZEN and system.self_update_allowed() and _update_due(path.parent, now):
        _start_logging(path)
        from update_check import updater

        if updater.self_update(path.parent):
            system.release_single_instance(lock)
            updater.restart()
            return

    data = load(path)
    # Only the .exe registers itself; from source this would point Windows startup at the dev Python. Nothing
    # is registered before the first start has asked.
    if system.FROZEN and data.settings.autostart is not None:
        try:
            system.sync_autostart(data.settings.autostart, data.settings.hour)
        except OSError:
            # The log file may not be open yet: most runs now skip the update check that opens it.
            _start_logging(path)
            import logging

            logging.getLogger(__name__).exception("Couldn't register the app to start with Windows")
    data.roll_over(now)
    if check_only:
        # A full-screen game or a presentation isn't interrupted. Today's reminder is then still to come, at the
        # next sign-in or daily check.
        if not data.should_remind(now) or system.user_is_busy():
            save(data, path)
            return
        data.last_reminded = now.date()
    save(data, path)

    # Imported only when the window opens: customtkinter was over half of a background check's run time.
    _start_logging(path)
    from update_check.app import open_window

    open_window(data, path, reminder=check_only)
