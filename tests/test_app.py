import struct

from update_check.app import ICON, _reuse
from update_check.state import Step


class FakeWidget:
    def __init__(self, step: Step):
        self.step = step
        self.destroyed = False

    def destroy(self) -> None:
        self.destroyed = True


def test_widgets_of_unchanged_steps_are_kept_in_the_new_order():
    a, b, c = Step("A", id="a"), Step("B", id="b"), Step("C", id="c")
    widgets = _reuse([], [a, b, c], FakeWidget)

    kept = _reuse(widgets, [c, a], FakeWidget)

    assert kept == [widgets[2], widgets[0]]
    assert widgets[1].destroyed
    assert not any(widget.destroyed for widget in kept)


def test_changed_step_gets_a_new_widget():
    old = _reuse([], [Step("A", id="a")], FakeWidget)
    renamed = Step("A renamed", id="a")

    new = _reuse(old, [renamed], FakeWidget)

    assert new[0] is not old[0]
    assert old[0].destroyed
    assert new[0].step is renamed


def test_identical_steps_each_keep_a_widget_pointing_at_their_own_step():
    first, second = Step("A", id="a"), Step("A", id="a")
    widgets = _reuse([], [first, second], FakeWidget)

    kept = _reuse(widgets, [second, first], FakeWidget)

    assert {id(widget) for widget in kept} == {id(widget) for widget in widgets}
    assert kept[0].step is second
    assert kept[1].step is first
    assert not any(widget.destroyed for widget in widgets)


def test_the_icon_has_the_sizes_windows_asks_for():
    data = ICON.read_bytes()
    reserved, kind, count = struct.unpack_from("<3H", data)
    assert (reserved, kind) == (0, 1)
    # Each 16-byte entry of the directory starts with the image's width, 0 standing for 256.
    widths = {data[6 + 16 * index] or 256 for index in range(count)}
    assert {16, 20, 24, 32, 40, 48, 64, 256} <= widths
