"""Start-up before the window opens: single instance, self-update, autostart and the reminder check."""

from pathlib import Path

from update_check import system
from update_check.state import data_file, load, local_now, save


def _start_logging(path: Path) -> None:
    """Open the log file. Called only from the paths that can log, because importing logging costs about
    a third of a background check's start-up and a check with nothing to report never writes a line.
    Safe to call twice: basicConfig does nothing once the root logger has a handler."""
    import logging

    logging.basicConfig(
        filename=path.with_name("update-check.log"),
        encoding="utf-8",
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        level=logging.INFO,
    )


def run(check_only: bool) -> None:
    """Open the checklist. With `check_only`, open it only when this week's check still needs a reminder."""
    path = data_file()
    now = local_now()
    lock = system.claim_single_instance()
    if lock is None:
        # The open window saves its own state, so only read it here.
        data = load(path)
        data.roll_over(now)
        if not check_only or data.should_remind(now):
            system.focus_window(system.TITLE)
        return

    path.parent.mkdir(parents=True, exist_ok=True)
    # Holding the lock means no other copy is running from the .exe that the update replaces. Only the .exe
    # updates itself, and importing the updater costs more than the rest of a background check put together,
    # so both are left out of runs from source and of the path that just focuses an already open window.
    if system.FROZEN:
        _start_logging(path)
        from update_check import updater

        if updater.self_update(path.parent):
            system.release_single_instance(lock)
            updater.restart()
            return

    data = load(path)
    if system.FROZEN:
        # Only the .exe registers itself; from source this would point Windows startup at the dev Python.
        try:
            system.sync_autostart(data.settings.autostart, data.settings.hour)
        except OSError:
            import logging

            logging.getLogger(__name__).exception("Couldn't register the app to start with Windows")
    data.roll_over(now)
    if check_only:
        if not data.should_remind(now):
            save(data, path)
            return
        data.last_reminded = now.date()
    save(data, path)

    # Imported only when the window opens: customtkinter was over half of a background check's run time.
    _start_logging(path)
    from update_check.app import open_window

    open_window(data, path, reminder=check_only)
