import hashlib
import urllib.error
from email.message import Message

import pytest

from update_check import updater
from update_check.updater import is_newer, verify

EXE = b"MZ" + bytes(100)


@pytest.mark.parametrize(
    ("tag", "installed", "newer"),
    [
        ("v0.2.0", "0.1.0", True),
        ("v0.10.0", "0.9.0", True),
        ("v0.1.0", "0.1.0", False),
        ("v0.1", "0.1.0", False),
        ("v0.0.9", "0.1.0", False),
        ("nightly", "0.1.0", False),
    ],
)
def test_only_newer_numeric_versions_count_as_updates(tag, installed, newer):
    assert is_newer(tag, installed) is newer


def test_verify_accepts_matching_download():
    digest = "sha256:" + hashlib.sha256(EXE).hexdigest()
    verify(EXE, {"name": "UpdateCheck.exe", "size": len(EXE), "digest": digest})


def test_verify_rejects_truncated_download():
    with pytest.raises(ValueError, match="incomplete"):
        verify(EXE, {"name": "UpdateCheck.exe", "size": len(EXE) + 1})


def test_verify_rejects_non_executable():
    with pytest.raises(ValueError, match="not a Windows executable"):
        verify(b"<html>", {"name": "UpdateCheck.exe", "size": 6})


def test_verify_rejects_digest_mismatch():
    with pytest.raises(ValueError, match="SHA-256"):
        verify(EXE, {"name": "UpdateCheck.exe", "size": len(EXE), "digest": "sha256:" + "0" * 64})


def test_verify_rejects_a_download_without_a_digest():
    with pytest.raises(ValueError, match="without a SHA-256 digest"):
        verify(EXE, {"name": "UpdateCheck.exe", "size": len(EXE)})


def _github_answers(monkeypatch, code):
    def urlopen(request, timeout):
        raise urllib.error.HTTPError(request.full_url, code, "error", Message(), None)

    monkeypatch.setattr(updater.urllib.request, "urlopen", urlopen)


def test_a_repository_without_releases_has_nothing_to_update_to(monkeypatch):
    _github_answers(monkeypatch, 404)
    assert updater._find_update("0.1.0") is None


def test_other_github_errors_still_count_as_failures(monkeypatch):
    _github_answers(monkeypatch, 503)
    with pytest.raises(urllib.error.HTTPError):
        updater._find_update("0.1.0")
