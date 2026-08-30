"""The weekly update checklist window."""

from dataclasses import astuple
from datetime import date
from pathlib import Path
from tkinter import messagebox

import customtkinter as ctk

from update_check import system
from update_check.state import (
    APPEARANCES,
    WEEKDAYS,
    State,
    Step,
    default_steps,
    local_now,
    save,
)
from update_check.system import TITLE

HOURS = [f"{hour:02d}:00" for hour in range(24)]

# (light, dark) color pairs
BG = ("#F3F4F6", "#111318")
CARD = ("#FFFFFF", "#1B1E24")
CARD_DONE = ("#F9FAFB", "#16181D")
BORDER = ("#E5E7EB", "#2A2E36")
TEXT = ("#111827", "#F3F4F6")
MUTED = ("#6B7280", "#9CA3AF")
ACCENT = ("#2563EB", "#3B82F6")
ACCENT_HOVER = ("#1D4ED8", "#2563EB")
ACCENT_SOFT = ("#BFDBFE", "#1E3A8A")
SUCCESS = ("#16A34A", "#22C55E")
SUBTLE = ("#E5E7EB", "#262A31")
SUBTLE_HOVER = ("#D1D5DB", "#323741")
SELECTED = ("#FFFFFF", "#3A3F48")


def _font(size: int, bold: bool = False) -> ctk.CTkFont:
    return ctk.CTkFont(family="Segoe UI", size=size, weight="bold" if bold else "normal")


def _secondary_button(master, **kwargs) -> ctk.CTkButton:
    return ctk.CTkButton(
        master, height=30, corner_radius=8, font=_font(13),
        fg_color=SUBTLE, hover_color=SUBTLE_HOVER, text_color=TEXT, **kwargs,
    )


def _primary_button(master, **kwargs) -> ctk.CTkButton:
    return ctk.CTkButton(
        master, height=34, corner_radius=8, font=_font(13, True),
        fg_color=ACCENT, hover_color=ACCENT_HOVER, text_color="#FFFFFF", **kwargs,
    )


def _card(master) -> ctk.CTkFrame:
    card = ctk.CTkFrame(master, fg_color=CARD, border_color=BORDER, border_width=1, corner_radius=12)
    card.pack(fill="x", pady=5)
    return card


def _section(master, text: str) -> None:
    ctk.CTkLabel(
        master, text=text.upper(), font=_font(11, True), text_color=MUTED, anchor="w", height=16,
    ).pack(fill="x", padx=4, pady=(18, 4))


def _day(moment: date) -> str:
    return f"{moment:%b} {moment.day}"


def _when(day: date, hour: int) -> str:
    return f"{day:%a}, {_day(day)} at {hour:02d}:00"


def _place(window: ctk.CTk | ctk.CTkToplevel, width: int, height: int) -> None:
    """Center the window on the work area, shrinking it on small screens so it clears the taskbar."""
    scaling = ctk.ScalingTracker.get_window_scaling(window)
    left, top, right, bottom = system.work_area()
    # Leave room for the title bar, which the geometry height doesn't include.
    height = min(height, round((bottom - top) / scaling) - 60)
    # Tk positions windows in physical pixels (winfo_screenwidth reports logical ones), and
    # CustomTkinter scales only the size, so the position is computed in physical pixels.
    x = left + max((right - left - round(width * scaling)) // 2, 0)
    y = top + max((bottom - top - round((height + 32) * scaling)) // 2, 0)
    window.geometry(f"{width}x{height}+{x}+{y}")


def _reuse(widgets: list, steps: list[Step], create) -> list:
    """Return one widget per step, in order. Building a widget takes tens of milliseconds, so the widgets of
    unchanged steps are kept; the rest are made with `create(step)` or destroyed."""
    unused: dict[tuple, list] = {}
    for widget in widgets:
        unused.setdefault(astuple(widget.step), []).append(widget)
    kept = []
    for step in steps:
        if matches := unused.get(astuple(step)):
            widget = matches.pop()
            widget.step = step
        else:
            widget = create(step)
        kept.append(widget)
    for leftovers in unused.values():
        for widget in leftovers:
            widget.destroy()
    return kept


def _pack_in_order(master, widgets: list, **options) -> None:
    """Pack `widgets` into `master` in this order. Only widgets that are out of place are moved: re-packing
    every card made reordering a step 6-15x slower."""
    slaves = master.pack_slaves()
    for index, widget in enumerate(widgets):
        if index < len(slaves) and slaves[index] is widget:
            continue
        if index:
            widget.pack(after=widgets[index - 1], **options)
        elif slaves:
            widget.pack(before=slaves[0], **options)
        else:
            widget.pack(**options)
        if widget in slaves:
            slaves.remove(widget)
        slaves.insert(index, widget)


class StepCard(ctk.CTkFrame):
    def __init__(self, master, step: Step, done: bool, on_toggle):
        # Colored up front rather than by _restyle(): recoloring a frame redraws it and all its children.
        super().__init__(
            master, fg_color=CARD_DONE if done else CARD, border_color=BORDER, border_width=1, corner_radius=12,
        )
        self.step = step
        self.on_toggle = on_toggle
        self.done = ctk.BooleanVar(value=done)
        self.grid_columnconfigure(1, weight=1)

        ctk.CTkCheckBox(
            self, text="", width=24, variable=self.done, command=self._toggled,
            checkbox_width=22, checkbox_height=22, corner_radius=6, border_width=2,
            fg_color=SUCCESS, hover_color=SUCCESS, border_color=MUTED, checkmark_color="#FFFFFF",
        ).grid(row=0, column=0, rowspan=2, padx=(16, 12), pady=16)

        bottom_pad = 0 if step.detail else 14
        self.title_label = ctk.CTkLabel(
            self, text=step.title, font=_font(15, True), text_color=MUTED if done else TEXT, anchor="w", height=22,
        )
        self.title_label.grid(row=0, column=1, sticky="ew", pady=(14, bottom_pad))
        self.detail_label = ctk.CTkLabel(
            self, text=step.detail, font=_font(12), text_color=MUTED, anchor="w", justify="left", height=18,
        )
        if step.detail:
            self.detail_label.grid(row=1, column=1, sticky="ew", pady=(0, 14))

        if step.action:
            _secondary_button(self, text="Open", width=64, command=self._open).grid(
                row=0, column=2, rowspan=2, padx=16,
            )

        for widget in (self, self.title_label, self.detail_label):
            widget.bind("<Button-1>", self._clicked)
        self.bind("<Configure>", self._wrap_detail)

    def _clicked(self, _event) -> None:
        self.done.set(not self.done.get())
        self._toggled()

    def _toggled(self) -> None:
        self._restyle()
        self.on_toggle(self.step.id, self.done.get())

    def set_done(self, done: bool) -> None:
        if done != self.done.get():
            self.done.set(done)
            self._restyle()

    def _restyle(self) -> None:
        done = self.done.get()
        self.configure(fg_color=CARD_DONE if done else CARD)
        self.title_label.configure(text_color=MUTED if done else TEXT)

    def _wrap_detail(self, event) -> None:
        scaling = ctk.ScalingTracker.get_widget_scaling(self)
        # Checkbox column, the Open button column and a little slack so the text never touches the button.
        reserved = 176 if self.step.action else 80
        self.detail_label.configure(wraplength=max(event.width / scaling - reserved, 120))

    def _open(self) -> None:
        try:
            system.open_action(self.step.action)
        except OSError as error:
            messagebox.showerror(TITLE, f"Couldn't open “{self.step.action}”.\n\n{error}", parent=self)


class App(ctk.CTk):
    def __init__(self, data: State, path: Path, reminder: bool):
        super().__init__(fg_color=BG)
        self.data = data
        self.path = path
        self.settings_window: SettingsWindow | None = None
        self.title(TITLE)
        _place(self, 560, 700)
        self.minsize(460, 520)
        # CustomTkinter polls the Windows theme every 30 ms, most of an idle window's CPU use.
        ctk.AppearanceModeTracker.update_loop_interval = 1000

        self._build_header()
        self._build_footer()
        self.steps_frame = ctk.CTkScrollableFrame(
            self, fg_color="transparent",
            scrollbar_button_color=SUBTLE, scrollbar_button_hover_color=SUBTLE_HOVER,
        )
        self.steps_frame.pack(fill="both", expand=True, padx=(20, 12), pady=(0, 12))
        self.cards: list[StepCard] = []
        self.empty_label = ctk.CTkLabel(
            self.steps_frame, text="No steps yet. Add some in Settings.", font=_font(13), text_color=MUTED,
        )

        self.render()
        self.after(60_000, self._tick)
        if reminder:
            self._bring_to_front()

    def _build_header(self) -> None:
        header = ctk.CTkFrame(self, fg_color="transparent")
        header.pack(fill="x", padx=28, pady=(24, 14))
        header.grid_columnconfigure(0, weight=1)

        ctk.CTkLabel(
            header, text="WEEKLY UPDATE CHECK", font=_font(11, True), text_color=ACCENT, anchor="w", height=16,
        ).grid(row=0, column=0, sticky="w")
        _secondary_button(header, text="Settings", width=84, command=self.open_settings).grid(
            row=0, column=1, rowspan=2, sticky="ne",
        )
        self.headline = ctk.CTkLabel(header, font=_font(26, True), text_color=TEXT, anchor="w", height=38)
        self.headline.grid(row=1, column=0, sticky="w")
        self.summary = ctk.CTkLabel(header, font=_font(13), text_color=MUTED, anchor="w", height=20)
        self.summary.grid(row=2, column=0, columnspan=2, sticky="w")
        self.progress = ctk.CTkProgressBar(header, height=8, corner_radius=4, fg_color=SUBTLE)
        self.progress.grid(row=3, column=0, columnspan=2, sticky="ew", pady=(14, 0))

    def _build_footer(self) -> None:
        footer = ctk.CTkFrame(self, fg_color=CARD, corner_radius=0)
        footer.pack(fill="x", side="bottom")
        ctk.CTkFrame(footer, height=1, corner_radius=0, fg_color=BORDER).pack(fill="x")
        inner = ctk.CTkFrame(footer, fg_color="transparent")
        inner.pack(fill="x", padx=28, pady=14)
        inner.grid_columnconfigure(1, weight=1)

        self.autostart_switch = ctk.CTkSwitch(
            inner, text="", width=44, switch_width=40, switch_height=22, command=self._autostart_toggled,
            fg_color=SUBTLE, progress_color=ACCENT_SOFT,
        )
        self.autostart_switch.grid(row=0, column=0, rowspan=2, padx=(0, 12))
        if system.autostart_enabled():
            self.autostart_switch.select()
        ctk.CTkLabel(
            inner, text="Automatic reminder", font=_font(13, True), text_color=TEXT, anchor="w", height=18,
        ).grid(row=0, column=1, sticky="w")
        self.schedule_label = ctk.CTkLabel(inner, font=_font(12), text_color=MUTED, anchor="w", height=16)
        self.schedule_label.grid(row=1, column=1, sticky="w")

        ctk.CTkLabel(
            inner, text="Last full check", font=_font(12), text_color=MUTED, anchor="e", height=18,
        ).grid(row=0, column=2, sticky="e")
        self.last_label = ctk.CTkLabel(inner, font=_font(13, True), text_color=TEXT, anchor="e", height=16)
        self.last_label.grid(row=1, column=2, sticky="e")

    def render(self) -> None:
        done = self.data.done
        self.cards = _reuse(
            self.cards, self.data.steps,
            lambda step: StepCard(self.steps_frame, step, step.id in done, self._step_toggled),
        )
        for card in self.cards:
            card.set_done(card.step.id in done)
        if self.cards:
            self.empty_label.pack_forget()
        else:
            self.empty_label.pack(pady=40)
        _pack_in_order(self.steps_frame, self.cards, fill="x", padx=(0, 8), pady=5)
        self._refresh_summary()

    def _refresh_summary(self) -> None:
        now = local_now()
        data = self.data
        settings = data.settings
        total = len(data.steps)
        done = sum(step.id in data.done for step in data.steps)

        if data.is_complete:
            self.headline.configure(text="All caught up")
            self.summary.configure(text=f"Nice work. Next check is {_when(data.next_reminder(now), settings.hour)}.")
            self.progress.configure(progress_color=SUCCESS)
        else:
            self.headline.configure(text="Time for updates")
            week = data.cycle_start or data.current_cycle_start(now)
            self.summary.configure(text=f"{done} of {total} steps done · week of {_day(week)}")
            self.progress.configure(progress_color=ACCENT)
        self.progress.set(done / total if total else 0)

        schedule = f"{WEEKDAYS[settings.weekday]}s at {settings.hour:02d}:00"
        reminders_on = bool(self.autostart_switch.get())
        # The knob overhangs the track, so it carries the on/off color to stay visible on the footer.
        knob = ACCENT if reminders_on else MUTED
        self.autostart_switch.configure(button_color=knob, button_hover_color=knob)
        if reminders_on:
            self.schedule_label.configure(text=f"{schedule}, or at sign-in if missed")
        else:
            self.schedule_label.configure(text=f"Off · turn on to be reminded {schedule}")
        self.last_label.configure(text=_day(data.last_completed) if data.last_completed else "Never")

    def _step_toggled(self, step_id: str, done: bool) -> None:
        self.data.set_done(step_id, done, local_now())
        self.save()
        self._refresh_summary()

    def _autostart_toggled(self) -> None:
        self.set_autostart(bool(self.autostart_switch.get()))

    def set_autostart(self, enabled: bool) -> None:
        try:
            if enabled:
                system.enable_autostart(self.data.settings.hour)
            else:
                system.disable_autostart()
        except OSError as error:
            messagebox.showerror(TITLE, f"Couldn't change the automatic reminder.\n\n{error}", parent=self)
        enabled = system.autostart_enabled()
        # Remembered so the .exe doesn't register itself again on its next start.
        self.data.settings.autostart = enabled
        self.save()
        if enabled:
            self.autostart_switch.select()
        else:
            self.autostart_switch.deselect()
        self._refresh_summary()

    def save(self) -> None:
        save(self.data, self.path)

    def open_settings(self) -> None:
        if self.settings_window and self.settings_window.winfo_exists():
            self.settings_window.deiconify()
            self.settings_window.focus_force()
            return
        self.settings_window = SettingsWindow(self)

    def _tick(self) -> None:
        if self.data.roll_over(local_now()):
            self.save()
            self.render()
        self.after(60_000, self._tick)

    def _bring_to_front(self) -> None:
        self.attributes("-topmost", True)
        self.focus_force()
        self.after(1500, lambda: self.attributes("-topmost", False))


class StepRow(ctk.CTkFrame):
    """A step in Settings, with buttons to move and remove it."""

    def __init__(self, master, step: Step, on_move, on_remove):
        super().__init__(master, fg_color=CARD, border_color=BORDER, border_width=1, corner_radius=12)
        self.step = step
        self.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            self, text=step.title, font=_font(14, True), text_color=TEXT, anchor="w", height=20,
        ).grid(row=0, column=0, sticky="ew", padx=(16, 8), pady=(10, 0))
        ctk.CTkLabel(
            self, text=step.action or "No open action", font=_font(12), text_color=MUTED, anchor="w", height=18,
        ).grid(row=1, column=0, sticky="ew", padx=(16, 8), pady=(0, 10))
        controls = ctk.CTkFrame(self, fg_color="transparent")
        controls.grid(row=0, column=1, rowspan=2, padx=(0, 12))
        self.up = _secondary_button(controls, text="↑", width=30, command=lambda: on_move(self.step, -1))
        self.down = _secondary_button(controls, text="↓", width=30, command=lambda: on_move(self.step, 1))
        self.up.pack(side="left", padx=2)
        self.down.pack(side="left", padx=2)
        _secondary_button(controls, text="Remove", width=72, command=lambda: on_remove(self.step)).pack(
            side="left", padx=(6, 0),
        )

    def set_position(self, first: bool, last: bool) -> None:
        for button, disabled in ((self.up, first), (self.down, last)):
            state = "disabled" if disabled else "normal"
            # Configuring a button redraws it, so only when its state changes.
            if button.cget("state") != state:
                button.configure(state=state)


class SettingsWindow(ctk.CTkToplevel):
    def __init__(self, app: App):
        super().__init__(app, fg_color=BG)
        self.app = app
        self.title("Settings")
        _place(self, 500, 680)
        self.minsize(420, 480)
        self.transient(app)
        # A CTkToplevel on Windows only takes focus once it has finished appearing.
        self.after(200, self.focus_force)
        ctk.AppearanceModeTracker.add(self._reshow_after_theme_change, self)
        settings = app.data.settings

        body = ctk.CTkScrollableFrame(
            self, fg_color="transparent",
            scrollbar_button_color=SUBTLE, scrollbar_button_hover_color=SUBTLE_HOVER,
        )
        body.pack(fill="both", expand=True, padx=(16, 8), pady=(4, 16))

        _section(body, "Reminder")
        reminder = _card(body)
        reminder.grid_columnconfigure((0, 1), weight=1)
        menu_style = {
            "height": 34, "corner_radius": 8, "font": _font(13), "dropdown_font": _font(13),
            "fg_color": SUBTLE, "button_color": SUBTLE, "button_hover_color": SUBTLE_HOVER, "text_color": TEXT,
            "dropdown_fg_color": CARD, "dropdown_hover_color": SUBTLE, "dropdown_text_color": TEXT,
            "command": self._schedule_changed,
        }
        self.day_menu = ctk.CTkOptionMenu(reminder, values=list(WEEKDAYS), **menu_style)
        self.day_menu.set(WEEKDAYS[settings.weekday])
        self.day_menu.grid(row=0, column=0, sticky="ew", padx=(16, 6), pady=(16, 6))
        self.hour_menu = ctk.CTkOptionMenu(reminder, values=HOURS, **menu_style)
        self.hour_menu.set(HOURS[settings.hour])
        self.hour_menu.grid(row=0, column=1, sticky="ew", padx=(6, 16), pady=(16, 6))
        ctk.CTkLabel(
            reminder, font=_font(12), text_color=MUTED, anchor="w", justify="left", wraplength=380,
            text="The checklist resets at this time each week. If it isn't finished, "
            "you're reminded again at most once a day.",
        ).grid(row=1, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 12))

        _section(body, "Appearance")
        appearance = ctk.CTkSegmentedButton(
            body, values=list(APPEARANCES), command=self._appearance_changed, height=34, font=_font(13),
            fg_color=SUBTLE, selected_color=SELECTED, selected_hover_color=SELECTED,
            unselected_color=SUBTLE, unselected_hover_color=SUBTLE_HOVER, text_color=TEXT,
        )
        appearance.set(settings.appearance)
        appearance.pack(fill="x", pady=5)

        _section(body, "Checklist steps")
        self.steps_frame = ctk.CTkFrame(body, fg_color="transparent")
        self.steps_frame.pack(fill="x")
        self.rows: list[StepRow] = []
        self._render_steps()

        _section(body, "Add a step")
        form = _card(body)
        entry_style = {
            "height": 36, "corner_radius": 8, "border_width": 1, "font": _font(13),
            "fg_color": BG, "border_color": BORDER, "text_color": TEXT, "placeholder_text_color": MUTED,
        }
        self.new_title = ctk.CTkEntry(form, placeholder_text="Title, e.g. Router firmware", **entry_style)
        self.new_detail = ctk.CTkEntry(form, placeholder_text="What to check (optional)", **entry_style)
        self.new_action = ctk.CTkEntry(
            form, placeholder_text="Open action (optional): URL, ms-settings: page or command", **entry_style,
        )
        for index, entry in enumerate((self.new_title, self.new_detail, self.new_action)):
            entry.pack(fill="x", padx=16, pady=(16 if index == 0 else 0, 8))
        buttons = ctk.CTkFrame(form, fg_color="transparent")
        buttons.pack(fill="x", padx=16, pady=(4, 16))
        _primary_button(buttons, text="Add step", width=110, command=self._add_step).pack(side="left")
        _secondary_button(buttons, text="Restore defaults", width=130, command=self._restore_defaults).pack(
            side="right",
        )

    def _render_steps(self) -> None:
        self.rows = _reuse(
            self.rows, self.app.data.steps,
            lambda step: StepRow(self.steps_frame, step, self._move_step, self._remove_step),
        )
        _pack_in_order(self.steps_frame, self.rows, fill="x", pady=5)
        for index, row in enumerate(self.rows):
            row.set_position(first=index == 0, last=index == len(self.rows) - 1)

    def _changed(self) -> None:
        self.app.save()
        self.app.render()
        self._render_steps()

    def _schedule_changed(self, _value: str) -> None:
        settings = self.app.data.settings
        settings.weekday = WEEKDAYS.index(self.day_menu.get())
        settings.hour = HOURS.index(self.hour_menu.get())
        self.app.data.roll_over(local_now())
        if system.autostart_enabled():
            # Re-register the daily check for the new hour.
            self.app.set_autostart(True)
        self.app.save()
        self.app.render()

    def _appearance_changed(self, value: str) -> None:
        self.app.data.settings.appearance = value
        ctk.set_appearance_mode(value)
        self.app.save()

    def _reshow_after_theme_change(self, _mode: str) -> None:
        # To recolor title bars, CustomTkinter hides each window and restores its previous state 5 ms
        # later. Hiding the main window first also hides this transient window, so it gets "restored"
        # as hidden and the Settings button seems dead. Show it again once that has run.
        self.after(50, self._show_if_hidden)

    def _show_if_hidden(self) -> None:
        if self.winfo_exists() and self.state() == "withdrawn":
            self.deiconify()

    def destroy(self) -> None:
        ctk.AppearanceModeTracker.remove(self._reshow_after_theme_change)
        super().destroy()

    def _add_step(self) -> None:
        title = self.new_title.get().strip()
        if not title:
            self.new_title.focus_set()
            return
        self.app.data.steps.append(Step(title, self.new_detail.get().strip(), self.new_action.get().strip()))
        for entry in (self.new_title, self.new_detail, self.new_action):
            entry.delete(0, "end")
        self._changed()

    def _move_step(self, step: Step, offset: int) -> None:
        steps = self.app.data.steps
        # By identity, since a hand-edited checklist can hold two identical steps.
        index = next(index for index, other in enumerate(steps) if other is step)
        steps[index], steps[index + offset] = steps[index + offset], steps[index]
        self._changed()

    def _remove_step(self, step: Step) -> None:
        self.app.data.steps.remove(step)
        self.app.data.done.discard(step.id)
        self._changed()

    def _restore_defaults(self) -> None:
        if messagebox.askyesno(TITLE, "Replace your checklist with the default steps?", parent=self):
            self.app.data.steps = default_steps()
            self._changed()


def open_window(data: State, path: Path, reminder: bool) -> None:
    ctk.set_appearance_mode(data.settings.appearance)
    App(data, path, reminder=reminder).mainloop()
