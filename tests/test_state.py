from datetime import date, datetime

from update_check.state import Settings, State, Step, load, save

# 2026-09-14 is a Monday.


def make_state(**settings) -> State:
    return State(steps=[Step("A", id="a"), Step("B", id="b")], settings=Settings(**settings))


def test_cycle_starts_at_most_recent_reminder_time():
    state = make_state(weekday=0, hour=10)
    assert state.current_cycle_start(datetime(2026, 9, 14, 10, 0)) == datetime(2026, 9, 14, 10)
    assert state.current_cycle_start(datetime(2026, 9, 16, 8, 30)) == datetime(2026, 9, 14, 10)
    assert state.current_cycle_start(datetime(2026, 9, 14, 9, 59)) == datetime(2026, 9, 7, 10)
    assert state.next_reminder(datetime(2026, 9, 16)) == datetime(2026, 9, 21, 10)


def test_new_cycle_clears_checks_but_keeps_history():
    state = make_state()
    state.roll_over(datetime(2026, 9, 14, 11))
    state.set_done("a", True, datetime(2026, 9, 14, 12))
    state.set_done("b", True, datetime(2026, 9, 14, 12))
    assert state.is_complete

    assert not state.roll_over(datetime(2026, 9, 21, 9, 59))
    assert state.roll_over(datetime(2026, 9, 21, 10))
    assert not state.done
    assert not state.is_complete
    assert state.last_completed == datetime(2026, 9, 14, 12)


def test_moving_reminder_to_earlier_day_keeps_progress():
    state = make_state(weekday=0)
    state.roll_over(datetime(2026, 9, 14, 11))
    state.set_done("a", True, datetime(2026, 9, 14, 11))
    state.settings.weekday = 6
    assert not state.roll_over(datetime(2026, 9, 15, 9))
    assert state.done == {"a"}


def test_unchecking_a_step_reopens_the_week():
    state = make_state()
    state.roll_over(datetime(2026, 9, 14, 11))
    state.set_done("a", True, datetime(2026, 9, 14, 11))
    state.set_done("b", True, datetime(2026, 9, 14, 11))
    state.set_done("b", False, datetime(2026, 9, 14, 12))
    assert not state.is_complete


def test_reminds_at_most_once_a_day_until_complete():
    state = make_state()
    now = datetime(2026, 9, 14, 10)
    state.roll_over(now)
    assert state.should_remind(now)

    state.last_reminded = now.date()
    assert not state.should_remind(datetime(2026, 9, 14, 18))
    assert state.should_remind(datetime(2026, 9, 15, 10))

    state.set_done("a", True, now)
    state.set_done("b", True, now)
    assert not state.should_remind(datetime(2026, 9, 16, 10))


def test_no_reminder_without_steps():
    state = State(steps=[])
    now = datetime(2026, 9, 14, 10)
    state.roll_over(now)
    assert not state.should_remind(now)


def test_save_and_load_round_trip(tmp_path):
    path = tmp_path / "nested" / "state.json"
    state = make_state(weekday=4, hour=18, appearance="Dark")
    state.roll_over(datetime(2026, 9, 18, 19))
    state.set_done("a", True, datetime(2026, 9, 18, 19))
    state.last_reminded = date(2026, 9, 18)

    save(state, path)

    assert load(path) == state


def test_missing_file_gives_default_checklist(tmp_path):
    state = load(tmp_path / "state.json")
    assert state == State()
    assert state.steps


def test_unreadable_file_is_backed_up(tmp_path):
    path = tmp_path / "state.json"
    path.write_text("{not json", encoding="utf-8")

    assert load(path) == State()
    assert (tmp_path / "state.json.bak").read_text(encoding="utf-8") == "{not json"
