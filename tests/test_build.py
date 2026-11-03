import re
from pathlib import Path

SPEC = Path(__file__).parents[1] / "UpdateCheck.spec"


def test_the_spec_builds_in_what_the_exe_needs():
    # Running PyInstaller on __main__.py rather than on the spec writes a new spec, without whatever its command
    # line left out.
    spec = SPEC.read_text(encoding="utf-8")
    # The updater reads the running version from the package metadata, and stays off without it.
    assert "copy_metadata('update-check')" in spec
    # The icon on the .exe, and next to app.py for the title bar and taskbar.
    assert re.search(r"icon=\[?'src/update_check/icon\.ico'\]?", spec)
    assert re.search(r"\('src/update_check/icon\.ico',\s*'update_check'\)", spec)
