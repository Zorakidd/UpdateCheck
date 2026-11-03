import json
from datetime import UTC, date, datetime, timedelta, timezone

from update_check.state import GRAPHICS_STEP, Settings, State, Step, load, save


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
    # Not asked yet: the first start asks before setting anything up.
    assert state.settings.autostart is None


def test_an_answer_about_the_reminder_is_kept(tmp_path):
    # Checklists saved before the first start asked have it on, and keep it on.
    path = tmp_path / "state.json"
    save(make_state(autostart=True), path)
    assert load(path).settings.autostart is True


def test_unreadable_file_is_backed_up(tmp_path):
    path = tmp_path / "state.json"
    path.write_text("{not json", encoding="utf-8")

    assert load(path) == State()
    assert (tmp_path / "state.json.bak").read_text(encoding="utf-8") == "{not json"


def test_checklist_saved_before_a_new_default_step_gets_it_once(tmp_path):
    path = tmp_path / "state.json"
    state = State()
    state.steps = [step for step in state.steps if step.id != GRAPHICS_STEP]
    save(state, path)
    data = json.loads(path.read_text(encoding="utf-8"))
    del data["offered_steps"]
    path.write_text(json.dumps(data), encoding="utf-8")

    loaded = load(path)
    ids = [step.id for step in loaded.steps]
    assert ids.index(GRAPHICS_STEP) == ids.index("drivers") + 1

    loaded.steps = [step for step in loaded.steps if step.id != GRAPHICS_STEP]
    save(loaded, path)
    assert GRAPHICS_STEP not in [step.id for step in load(path).steps]


def test_new_default_step_goes_after_the_last_earlier_default_step_or_at_the_end():
    offered = {"windows-update", "drivers", "store", "winget", "defender", "backup"}
    after_windows_update = State(steps=[Step("Windows Update", id="windows-update"), Step("Mine", id="mine")])
    only_custom = State(steps=[Step("Mine", id="mine")])
    for state in (after_windows_update, only_custom):
        state.offered_steps = set(offered)
        state.add_new_default_steps()

    assert [step.id for step in after_windows_update.steps] == ["windows-update", GRAPHICS_STEP, "storage", "mine"]
    assert [step.id for step in only_custom.steps] == ["mine", GRAPHICS_STEP, "storage"]


def test_ticking_a_step_records_the_day_it_was_done():
    state = make_state()
    state.roll_over(at(14, 10))
    state.set_done("a", True, at(16, 9))
    assert state.last_done == {"a": date(2026, 9, 16)}

    # Un-ticking is a correction to this week, not a claim that it was never done.
    state.set_done("a", False, at(16, 9))
    assert state.last_done == {"a": date(2026, 9, 16)}


def test_last_done_and_the_streak_survive_a_round_trip(tmp_path):
    state = make_state()
    state.roll_over(at(14, 10))
    state.set_done("a", True, at(14, 11))
    state.set_done("b", True, at(14, 11))
    path = tmp_path / "state.json"
    save(state, path)

    loaded = load(path)
    assert loaded.last_done == {"a": date(2026, 9, 14), "b": date(2026, 9, 14)}
    assert loaded.completed_weeks == [date(2026, 9, 14)]


def test_a_checklist_from_before_these_were_recorded_still_loads(tmp_path):
    path = tmp_path / "state.json"
    path.write_text(json.dumps({"steps": [], "settings": {}}), encoding="utf-8")
    state = load(path)
    assert state.last_done == {}
    assert state.completed_weeks == []


def test_finishing_a_week_adds_it_to_the_streak():
    state = make_state(weekday=0, hour=10)
    state.completed_weeks = [date(2026, 8, 31), date(2026, 9, 7)]
    state.roll_over(at(14, 10))
    # This week isn't finished yet, which doesn't break the run behind it.
    assert state.streak(at(14, 10)) == 2

    state.set_done("a", True, at(14, 11))
    state.set_done("b", True, at(14, 11))
    assert state.streak(at(14, 11)) == 3


def test_a_missed_week_ends_the_streak():
    state = make_state(weekday=0, hour=10)
    state.completed_weeks = [date(2026, 8, 24), date(2026, 9, 7)]
    state.roll_over(at(14, 10))
    assert state.streak(at(14, 10)) == 1


def test_un_ticking_takes_this_week_back_off_the_streak():
    state = make_state(weekday=0, hour=10)
    state.roll_over(at(14, 10))
    state.set_done("a", True, at(14, 11))
    state.set_done("b", True, at(14, 11))
    assert state.completed_weeks == [date(2026, 9, 14)]

    state.set_done("b", False, at(14, 12))
    assert state.completed_weeks == []


def test_moving_the_reminder_day_keeps_the_streak():
    state = make_state(weekday=0, hour=10)
    # Three Mondays finished in a row.
    state.completed_weeks = [date(2026, 8, 31), date(2026, 9, 7), date(2026, 9, 14)]
    # The reminder moves to Wednesdays, so this week starts on the 16th and the weeks behind it no longer
    # line up with the days they were recorded on.
    state.settings.weekday = 2
    assert state.current_cycle_start(at(17, 10)) == date(2026, 9, 16)
    assert state.streak(at(17, 10)) == 3
