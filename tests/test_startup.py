import subprocess
import sys


def test_background_check_does_not_import_the_window_toolkit():
    # Importing customtkinter was over half of a background check's run time, and most checks never open the window.
    code = "import sys, update_check.startup; sys.exit('customtkinter' in sys.modules)"
    assert subprocess.run([sys.executable, "-c", code], check=False).returncode == 0
