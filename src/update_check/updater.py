"""Self-update of the single-file .exe from the latest GitHub release."""

import contextlib
import hashlib
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import urllib.request
from importlib import metadata
from pathlib import Path
from typing import Any

from update_check.system import FROZEN, USER_AGENT

# The repository whose releases carry the .exe, tagged with the version, e.g. "v1.2.0".
GITHUB_REPO = "Zorakidd/Update-check"
RELEASES_URL = os.environ.get(
    "UPDATE_CHECK_RELEASES_URL", f"https://api.github.com/repos/{GITHUB_REPO}/releases/latest"
)

log = logging.getLogger(__name__)


def current_version() -> str | None:
    """The running version, or None if the build didn't bundle the package metadata."""
    try:
        return metadata.version("update-check")
    except metadata.PackageNotFoundError:
        return None


def parse_version(text: str) -> tuple[int, ...] | None:
    """'v1.2.0' -> (1, 2). Trailing zeros are dropped so 'v1.2' and '1.2.0' compare equal."""
    match = re.fullmatch(r"v?(\d+(?:\.\d+)*)", text.strip())
    if match is None:
        return None
    parts = [int(part) for part in match.group(1).split(".")]
    while len(parts) > 1 and parts[-1] == 0:
        parts.pop()
    return tuple(parts)


def is_newer(tag: str, installed: str) -> bool:
    latest, current = parse_version(tag), parse_version(installed)
    return latest is not None and current is not None and latest > current


def self_update(data_dir: Path) -> bool:
    """Replace the running .exe with a newer release. Returns True if the caller should restart."""
    if not FROZEN:
        return False
    exe = Path(sys.executable)
    # Left behind by the previous update, and still locked until that process has exited.
    with contextlib.suppress(OSError):
        exe.with_name(exe.name + ".old").unlink(missing_ok=True)

    installed = current_version()
    if installed is None:
        log.error("Updates are off: the .exe was built without --copy-metadata update-check")
        return False
    marker = data_dir / "last-update.txt"
    try:
        found = _find_update(installed)
        if found is None:
            return False
        tag, asset = found
        if marker.is_file() and marker.read_text(encoding="utf-8").strip() == tag:
            # Without this, a release whose .exe reports an older version is downloaded on every start.
            log.error("Not reinstalling %s: it was installed before but reports version %s", tag, installed)
            return False
        _install(asset, exe)
        marker.write_text(tag, encoding="utf-8")
    except (OSError, ValueError, KeyError) as error:
        log.warning("Update failed: %s", error)
        return False
    log.info("Updated from %s to %s", installed, tag)
    return True


def _find_update(installed: str) -> tuple[str, dict[str, Any]] | None:
    request = urllib.request.Request(
        RELEASES_URL, headers={"User-Agent": USER_AGENT, "Accept": "application/vnd.github+json"}
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        release = json.load(response)
    tag = release.get("tag_name", "")
    if not is_newer(tag, installed):
        return None
    for asset in release.get("assets", []):
        if asset.get("name", "").lower().endswith(".exe"):
            return tag, asset
    log.warning("Release %s has no .exe attached", tag)
    return None


def _install(asset: dict[str, Any], exe: Path) -> None:
    new = exe.with_name(exe.name + ".new")
    old = exe.with_name(exe.name + ".old")
    request = urllib.request.Request(asset["browser_download_url"], headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response, new.open("wb") as file:
        shutil.copyfileobj(response, file)
    try:
        verify(new.read_bytes(), asset)
    except ValueError:
        new.unlink()
        raise
    # Windows won't overwrite or delete a running .exe, but it does allow renaming it.
    exe.replace(old)
    try:
        new.replace(exe)
    except OSError:
        old.replace(exe)
        raise


def verify(content: bytes, asset: dict[str, Any]) -> None:
    """Reject a truncated, corrupted or non-executable download."""
    name = asset.get("name", "download")
    if len(content) != asset.get("size", len(content)) or not content.startswith(b"MZ"):
        message = f"{name} is incomplete or not a Windows executable"
        raise ValueError(message)
    # GitHub publishes a "sha256:<hex>" digest for each release asset.
    algorithm, _, expected = (asset.get("digest") or "").partition(":")
    if algorithm == "sha256" and hashlib.sha256(content).hexdigest() != expected:
        message = f"{name} doesn't match the release's SHA-256 digest"
        raise ValueError(message)


def restart() -> None:
    """Start the newly installed .exe with the same arguments. The caller must exit right after."""
    # Starts the .exe as an independent app. Otherwise PyInstaller's bootloader treats it as a child of
    # this process and reuses this process's temporary folder, which is deleted when this process exits.
    env = {**os.environ, "PYINSTALLER_RESET_ENVIRONMENT": "1"}
    subprocess.Popen([sys.executable, *sys.argv[1:]], env=env)
