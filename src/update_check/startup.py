"""Start-up before the window opens: single instance, self-update, autostart and the reminder check."""

import logging

from update_check import system, updater
from update_check.state import data_file, load, local_now, save

log = logging.getLogger(__name__)


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
    logging.basicConfig(
        filename=path.with_name("update-check.log"),
        encoding="utf-8",
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        level=logging.INFO,
    )
    # Holding the lock means no other copy is running from the .exe that the update replaces.
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
            log.exception("Couldn't register the app to start with Windows")
    data.roll_over(now)
    if check_only:
        if not data.should_remind(now):
            save(data, path)
            return
        data.last_reminded = now.date()
    save(data, path)

    # Imported only when the window opens: customtkinter was over half of a background check's run time.
    from update_check.app import open_window

    open_window(data, path, reminder=check_only)
