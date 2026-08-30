from datetime import UTC, date, datetime, timedelta, timezone

from update_check.state import Settings, State, Step, load, save


def at(day: int, hour: int = 0, minute: int = 0) -> datetime:
    """A time in September 2026. The 14th is a Monday."""
    return datetime(2026, 9, day, hour, minute, tzinfo=UTC)


def make_state(**settings) -> State:
    return State(steps=[Step("A", id="a"), Step("B", id="b")], settings=Settings(**settings))


def test_cycle_starts_on_most_recent_reminder_day_after_its_hour():
    state = make_state(weekday=0, hour=10)
    assert state.current_cycle_start(at(14, 10)) == date(2026, 9, 14)
    assert state.current_cycle_start(at(16, 8, 30)) == date(2026, 9, 14)
    assert state.current_cycle_start(at(14, 9, 59)) == date(2026, 9, 7)
    assert state.next_reminder(at(16)) == date(2026, 9, 21)


def test_new_cycle_clears_checks_but_keeps_history():
    state = make_state()
    state.roll_over(at(14, 11))
    state.set_done("a", True, at(14, 12))
    state.set_done("b", True, at(14, 12))
    assert state.is_complete

    assert not state.roll_over(at(21, 9, 59))
    assert state.roll_over(at(21, 10))
    assert not state.done
    assert not state.is_complete
    assert state.last_completed == at(14, 12)


def test_daylight_saving_change_does_not_restart_the_week():
    summer, winter = timezone(timedelta(hours=2)), timezone(timedelta(hours=1))
    state = make_state(weekday=0, hour=10)
    state.roll_over(datetime(2026, 10, 19, 11, tzinfo=summer))
    state.set_done("a", True, datetime(2026, 10, 19, 11, tzinfo=summer))

    assert not state.roll_over(datetime(2026, 10, 26, 9, 30, tzinfo=winter))
    assert state.done == {"a"}


def test_moving_reminder_to_earlier_day_keeps_progress():
    state = make_state(weekday=0)
    state.roll_over(at(14, 11))
    state.set_done("a", True, at(14, 11))
    state.settings.weekday = 6
    assert not state.roll_over(at(15, 9))
    assert state.done == {"a"}


def test_unchecking_a_step_reopens_the_week():
    state = make_state()
    state.roll_over(at(14, 11))
    state.set_done("a", True, at(14, 11))
    state.set_done("b", True, at(14, 11))
    state.set_done("b", False, at(14, 12))
    assert not state.is_complete


def test_reminds_at_most_once_a_day_until_complete():
    state = make_state()
    now = at(14, 10)
    state.roll_over(now)
    assert state.should_remind(now)

    state.last_reminded = now.date()
    assert not state.should_remind(at(14, 18))
    assert state.should_remind(at(15, 10))

    state.set_done("a", True, now)
    state.set_done("b", True, now)
    assert not state.should_remind(at(16, 10))


def test_no_reminder_without_steps():
    state = State(steps=[])
    state.roll_over(at(14, 10))
    assert not state.should_remind(at(14, 10))


def test_save_and_load_round_trip(tmp_path):
    path = tmp_path / "nested" / "state.json"
    state = make_state(weekday=4, hour=18, appearance="Dark", autostart=False)
    state.roll_over(at(18, 19))
    state.set_done("a", True, at(18, 19))
    state.last_reminded = date(2026, 9, 18)

    save(state, path)

    assert load(path) == state


def test_loads_cycle_start_saved_as_timestamp(tmp_path):
    path = tmp_path / "state.json"
    save(make_state(), path)
    path.write_text(path.read_text(encoding="utf-8").replace('"cycle_start": null', '"cycle_start": "2026-09-14T10:00:00"'))

    assert load(path).cycle_start == date(2026, 9, 14)


def test_missing_file_gives_default_checklist(tmp_path):
    state = load(tmp_path / "state.json")
    assert state == State()
    assert state.steps
    assert state.settings.autostart


def test_unreadable_file_is_backed_up(tmp_path):
    path = tmp_path / "state.json"
    path.write_text("{not json", encoding="utf-8")

    assert load(path) == State()
    assert (tmp_path / "state.json.bak").read_text(encoding="utf-8") == "{not json"
