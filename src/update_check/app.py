"""The weekly update checklist window."""

import logging
from datetime import date
from pathlib import Path
from tkinter import messagebox

import customtkinter as ctk

from update_check import system, updater
from update_check.state import (
    APPEARANCES,
    WEEKDAYS,
    State,
    Step,
    data_file,
    default_steps,
    load,
    local_now,
    save,
)

log = logging.getLogger(__name__)

TITLE = "Update Check"
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


class StepCard(ctk.CTkFrame):
    def __init__(self, master, step: Step, done: bool, on_toggle):
        super().__init__(master, fg_color=CARD, border_color=BORDER, border_width=1, corner_radius=12)
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
        self.title_label = ctk.CTkLabel(self, text=step.title, font=_font(15, True), anchor="w", height=22)
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
        self._restyle()

    def _clicked(self, _event) -> None:
        self.done.set(not self.done.get())
        self._toggled()

    def _toggled(self) -> None:
        self._restyle()
        self.on_toggle(self.step.id, self.done.get())

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

        self._build_header()
        self._build_footer()
        self.steps_frame = ctk.CTkScrollableFrame(
            self, fg_color="transparent",
            scrollbar_button_color=SUBTLE, scrollbar_button_hover_color=SUBTLE_HOVER,
        )
        self.steps_frame.pack(fill="both", expand=True, padx=(20, 12), pady=(0, 12))

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
        for child in self.steps_frame.winfo_children():
            child.destroy()
        for step in self.data.steps:
            StepCard(self.steps_frame, step, step.id in self.data.done, self._step_toggled).pack(
                fill="x", padx=(0, 8), pady=5,
            )
        if not self.data.steps:
            ctk.CTkLabel(
                self.steps_frame, text="No steps yet. Add some in Settings.", font=_font(13), text_color=MUTED,
            ).pack(pady=40)
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
            self.summary.configure(text=f"{done} of {total} steps done · week of {_day(data.cycle_start)}")
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
        for child in self.steps_frame.winfo_children():
            child.destroy()
        steps = self.app.data.steps
        for index, step in enumerate(steps):
            row = _card(self.steps_frame)
            row.grid_columnconfigure(0, weight=1)
            ctk.CTkLabel(
                row, text=step.title, font=_font(14, True), text_color=TEXT, anchor="w", height=20,
            ).grid(row=0, column=0, sticky="ew", padx=(16, 8), pady=(10, 0))
            ctk.CTkLabel(
                row, text=step.action or "No open action", font=_font(12), text_color=MUTED, anchor="w", height=18,
            ).grid(row=1, column=0, sticky="ew", padx=(16, 8), pady=(0, 10))
            controls = ctk.CTkFrame(row, fg_color="transparent")
            controls.grid(row=0, column=1, rowspan=2, padx=(0, 12))
            up = _secondary_button(controls, text="↑", width=30, command=lambda i=index: self._move_step(i, -1))
            down = _secondary_button(controls, text="↓", width=30, command=lambda i=index: self._move_step(i, 1))
            up.configure(state="disabled" if index == 0 else "normal")
            down.configure(state="disabled" if index == len(steps) - 1 else "normal")
            up.pack(side="left", padx=2)
            down.pack(side="left", padx=2)
            _secondary_button(controls, text="Remove", width=72, command=lambda s=step: self._remove_step(s)).pack(
                side="left", padx=(6, 0),
            )

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

    def _add_step(self) -> None:
        title = self.new_title.get().strip()
        if not title:
            self.new_title.focus_set()
            return
        self.app.data.steps.append(Step(title, self.new_detail.get().strip(), self.new_action.get().strip()))
        for entry in (self.new_title, self.new_detail, self.new_action):
            entry.delete(0, "end")
        self._changed()

    def _move_step(self, index: int, offset: int) -> None:
        steps = self.app.data.steps
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


def run(check_only: bool) -> None:
    """Open the checklist. With `check_only`, open it only when this week's check still needs a reminder."""
    path = data_file()
    now = local_now()
    lock = system.claim_single_instance()
    if lock is None:
        # The open window saves its own state, so only read it here.
        data = load(path)
        data.roll_over(now)
        if not check_only or data.should_remind(now):
            system.focus_window(TITLE)
        return

    path.parent.mkdir(parents=True, exist_ok=True)
    logging.basicConfig(
        filename=path.with_name("update-check.log"),
        encoding="utf-8",
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        level=logging.INFO,
    )
    # Holding the lock means no other copy is running from the .exe that the update replaces.
    if updater.self_update(path.parent):
        system.release_single_instance(lock)
        updater.restart()
        return

    data = load(path)
    if system.FROZEN:
        # Only the .exe registers itself; from source this would point Windows startup at the dev Python.
        try:
            system.sync_autostart(data.settings.autostart, data.settings.hour)
        except OSError:
            log.exception("Couldn't register the app to start with Windows")
    data.roll_over(now)
    if check_only:
        if not data.should_remind(now):
            save(data, path)
            return
        data.last_reminded = now.date()
    save(data, path)

    ctk.set_appearance_mode(data.settings.appearance)
    App(data, path, reminder=check_only).mainloop()
