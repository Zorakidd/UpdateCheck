import time

from update_check.pending import (
    CHECK_FAILED,
    CHECKING,
    NOTHING_PENDING,
    PENDING,
    PendingChecks,
    count_winget_table,
    describe,
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


def test_an_absurd_number_of_rows_is_not_an_answer():
    rows = "".join(f"Package{index}   Id{index}   1.0   2.0   winget\n" for index in range(1200))
    assert count_winget_table("-" * 40 + "\n" + rows) is None


def test_counts_are_described_in_the_words_of_the_step():
    assert describe("winget", 1) == "1 upgrade waiting"
    assert describe("winget", 4) == "4 upgrades waiting"
    assert describe("windows-update", 1) == "1 update waiting"
    assert describe("windows-update", 0) == "0 updates waiting"


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
