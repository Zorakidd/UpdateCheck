import threading
import time
from datetime import date

from update_check.pending import (
    CHECK_FAILED,
    CHECKING,
    NOTHING_PENDING,
    PENDING,
    Found,
    PendingChecks,
    SharedSearch,
    WindowsUpdateSearch,
    count_winget_table,
    describe,
    parse_search,
)

# winget prints its table in whatever language Windows is set to; this is the German one.
TABLE = (
    "Name                                ID                         Version Verfügbar Quelle\n"
    "---------------------------------------------------------------------------------------\n"
    "Mozilla Firefox                     Mozilla.Firefox            145.0.1 146.0     winget\n"
    "GitHub CLI                          GitHub.cli                 2.100.0 2.101.0   winget\n"
    "Microsoft GameInput                 Microsoft.GameInput        3.4.218 3.5.270   winget\n"
    # Every column of this row is filled exactly, so only single spaces separate most of them.
    "Microsoft Visual Studio Code (User) Microsoft.VisualStudioCode 1.137.0 1.138.0   winget\n"
    "4 Aktualisierungen verfügbar.\n"
)


def wait_for(checks: PendingChecks) -> None:
    deadline = time.monotonic() + 5
    while checks.busy and time.monotonic() < deadline:
        checks.collect()
        time.sleep(0.01)


def test_rows_are_counted_whatever_language_winget_speaks():
    assert count_winget_table(TABLE) == 4


def test_a_row_with_the_widest_entry_in_every_column_still_counts():
    # What winget said on 2026-11-03: Audacity fills every column, so its row has only single spaces.
    output = (
        "Name         ID                Version         Verfügbar       Quelle\n"
        "---------------------------------------------------------------------\n"
        "Audacity 4.0 Audacity.Audacity 4.0.1.262721353 4.0.2.263060742 winget\n"
        "Obsidian     Obsidian.Obsidian 1.14.5          1.15.0          winget\n"
        "2 Aktualisierungen verfügbar.\n"
    )
    assert count_winget_table(output) == 2


def test_rows_from_different_sources_count():
    output = (
        "Name      Id                Version   Available Source\n"
        "------------------------------------------------------\n"
        "Notepad++ Notepad++.Notepad++ 8.6     8.7       winget\n"
        "Spotify   9NCBCSZSJRSB      1.2.40    1.2.45    msstore\n"
        "2 upgrades available.\n"
    )
    assert count_winget_table(output) == 2


def test_wide_characters_take_two_columns():
    # A long Japanese name: 15 characters taking 30 columns, so the row is as wide as the rule in the console
    # although it has far fewer characters.
    name = "日本語の長いアプリケーション名"
    row = name + " " * 24 + "winget"
    output = f"名前\n{'-' * 60}\n{row}\n1 個のアップグレードが利用可能です。\n"
    assert len(row) < 60 - 12
    assert count_winget_table(output) == 1


def test_a_wordier_summary_is_still_not_counted_as_a_row():
    wordy = TABLE.replace(
        "4 Aktualisierungen verfügbar.",
        "4 Aktualisierungen sind verfügbar und können jetzt installiert werden.",
    )
    assert count_winget_table(wordy) == 4


def test_no_table_means_nothing_to_upgrade():
    assert count_winget_table("Keine installierten Pakete gefunden.\n") == 0


def test_no_output_at_all_is_not_an_answer():
    assert count_winget_table("") is None


def test_a_failing_winget_is_not_taken_for_nothing_to_upgrade():
    # What winget said, and the code it exited with, when asked to search a source that doesn't exist.
    output = "Keine Quellen entsprechen dem angegebenen Wert: gibtsnicht\nDie konfigurierten Quellen sind:\n  winget\n"
    assert count_winget_table(output, 0x8A150012) is None


def test_the_rows_of_a_table_count_whatever_the_exit_code():
    # Packages winget did list are waiting, whatever else went wrong.
    assert count_winget_table(TABLE, 0x8A15000F) == 4


def test_an_absurd_number_of_rows_is_not_an_answer():
    rows = "".join(f"Package{index}   Id{index}   1.0   2.0   winget\n" for index in range(1200))
    assert count_winget_table("-" * 40 + "\n" + rows) is None


def test_counts_are_described_in_the_words_of_the_step():
    assert describe("winget", 1) == "1 upgrade waiting"
    assert describe("winget", 4) == "4 upgrades waiting"
    assert describe("windows-update", 1) == "1 update waiting"
    assert describe("windows-update", 0) == "0 updates waiting"
    assert describe("drivers", 2) == "2 driver updates waiting"


def test_the_windows_update_search_is_read_strictly():
    assert parse_search("3 1 2026-11-03\n") == WindowsUpdateSearch(3, 1, date(2026, 11, 3))
    # Windows can't tell when it last installed updates.
    assert parse_search("0 0 -") == WindowsUpdateSearch(0, 0, None)
    assert parse_search("") is None
    assert parse_search("Zugriff verweigert") is None
    assert parse_search("3 1") is None
    assert parse_search("5000 0 2026-11-03") is None


def test_both_windows_update_steps_share_one_search():
    searches = []

    def search() -> WindowsUpdateSearch:
        searches.append(1)
        time.sleep(0.1)
        return WindowsUpdateSearch(2, 1, date(2026, 11, 3))

    shared = SharedSearch(search)
    results = []
    threads = [threading.Thread(target=lambda: results.append(shared())) for _ in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert len(searches) == 1
    assert results == [WindowsUpdateSearch(2, 1, date(2026, 11, 3))] * 2


def test_a_week_later_windows_update_is_searched_again(monkeypatch):
    searches = []
    shared = SharedSearch(lambda: searches.append(1) or None)
    shared()
    monkeypatch.setattr(SharedSearch, "FRESH_FOR", 0)
    shared()
    assert len(searches) == 2


def test_the_day_windows_last_installed_updates_comes_with_the_count():
    checks = PendingChecks(lookups={"windows-update": lambda: Found(2, date(2026, 11, 3))})
    assert checks.last_installed("windows-update") is None
    checks.refresh({"windows-update"})
    wait_for(checks)
    assert checks.status("windows-update") == (PENDING, 2)
    assert checks.last_installed("windows-update") == date(2026, 11, 3)


def test_a_count_is_looked_up_in_the_background():
    checks = PendingChecks(lookups={"winget": lambda: 4})

    assert checks.refresh({"winget"})
    assert checks.status("winget") == (CHECKING, 0)
    wait_for(checks)
    assert checks.status("winget") == (PENDING, 4)


def test_nothing_waiting_is_its_own_answer():
    checks = PendingChecks(lookups={"winget": lambda: 0})
    checks.refresh({"winget"})
    wait_for(checks)
    assert checks.status("winget") == (NOTHING_PENDING, 0)


def test_an_unreadable_answer_counts_as_a_failed_check():
    checks = PendingChecks(lookups={"winget": lambda: None})
    checks.refresh({"winget"})
    wait_for(checks)
    assert checks.status("winget") == (CHECK_FAILED, 0)


def test_a_tool_that_blows_up_counts_as_a_failed_check():
    def explode() -> int:
        raise OSError("winget is not installed")

    checks = PendingChecks(lookups={"winget": explode})
    checks.refresh({"winget"})
    wait_for(checks)
    assert checks.status("winget") == (CHECK_FAILED, 0)


def test_a_step_the_checklist_no_longer_has_is_not_looked_up():
    asked = []
    checks = PendingChecks(lookups={"winget": lambda: asked.append("winget") or 0})

    assert not checks.refresh({"backup"})
    assert asked == []


def test_failed_lookup_is_retried_after_a_while(monkeypatch):
    attempts = []

    def look_up() -> int:
        attempts.append(1)
        raise OSError("offline")

    checks = PendingChecks(lookups={"winget": look_up})
    checks.refresh({"winget"})
    wait_for(checks)
    assert checks.status("winget") == (CHECK_FAILED, 0)

    checks.refresh({"winget"})
    assert len(attempts) == 1

    monkeypatch.setattr(PendingChecks, "RETRY_AFTER", 0)
    checks.refresh({"winget"})
    wait_for(checks)
    assert len(attempts) == 2
