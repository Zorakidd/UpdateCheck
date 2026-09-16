import subprocess
import sys
from datetime import timedelta

from update_check.startup import _UPDATE_INTERVAL, _update_due
from update_check.state import local_now


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
