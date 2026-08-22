# Update Check

A small Windows app that reminds you once a week to work through a checklist of update steps:
Windows Update, driver updates, Store apps, winget, Defender and backups by default. It ships as a
single .exe that starts with Windows and keeps itself up to date from GitHub releases.

## How it works

- Tick steps off as you go. **Open** jumps to the matching settings page or runs the step's command.
- The checklist resets every week at the reminder day and time chosen in **Settings**, where you can
  also add, remove and reorder steps and switch between light and dark mode.
- **Starts with Windows.** When the .exe starts, it registers a sign-in entry for your user and a daily
  Task Scheduler task at the reminder hour (no admin rights needed), and re-registers them if the .exe
  has moved. Both run `UpdateCheck.exe --check`, which opens the window only while this week's checklist
  is unfinished, at most once a day. The **Automatic reminder** switch in the footer turns this off.
- **Updates itself.** On every start the .exe asks GitHub for the latest release of
  `Zorakidd/Update-check` (`GITHUB_REPO` in [updater.py](src/update_check/updater.py)). If its tag is a
  newer version and an .exe is attached, the app downloads it, checks its size and SHA-256 digest, swaps
  it in for the running .exe and restarts with the same arguments. Failures are logged and never stop
  the app.

Steps and progress are stored in `%APPDATA%\UpdateCheck\state.json`, next to `update-check.log`. Set
`UPDATE_CHECK_DATA` to use a different state file.

## Develop

```
uv run update-check
uv run pytest
uvx ruff check .
```

Running from source never updates itself or registers with Windows startup on its own; the footer
switch still works.

## Build and release

1. Bump `version` in `pyproject.toml`.
2. Build the single-file .exe:

   ```
   uv run --with pyinstaller pyinstaller --onefile --windowed --name UpdateCheck --paths src --copy-metadata update-check src/update_check/__main__.py
   ```

   `--copy-metadata update-check` is required: the updater reads the running version from it and
   stays off without it.
3. Publish a GitHub release tagged `v<version>` (for example `v0.2.0`) with `dist/UpdateCheck.exe`
   attached. The repository has to be public, since the updater doesn't authenticate.
