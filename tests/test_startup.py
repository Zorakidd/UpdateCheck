import subprocess
import sys


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
