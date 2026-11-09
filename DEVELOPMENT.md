# Developing Update Check

```
cargo run
cargo test
cargo clippy --all-targets
```

It builds with stable Rust (edition 2024) and either Windows toolchain; [.cargo/config.toml](.cargo/config.toml)
explains the linker the GNU one needs. The window is drawn with egui on Direct3D 12.

A debug build never updates itself or registers with Windows startup on its own, and doesn't ask about the
reminder on its first start; the footer switch still works. For testing the window without disturbing the
desktop, `UPDATE_CHECK_WINDOW_POS=x,y` opens it at that position without taking the focus (for example
`-20000,-20000`, off screen), `UPDATE_CHECK_RELEASES_URL` points the updater elsewhere, and
`UPDATE_CHECK_DATA` uses a different state file.

The icon, [icon.ico](assets/icon.ico), is drawn by `uv run --no-project --with pillow python tools/make_icon.py`.
[build.rs](build.rs) embeds it in the .exe with the version information and the application manifest,
without a resource compiler.

## How the app reads Windows

- **What's waiting.** winget prints its table in the PC's own language and Windows Update answers only
  through COM; neither has a machine-readable count. Both are read defensively, and a step says nothing
  rather than guess. One Windows Update search serves both of its steps and the PC info tab.
- **Graphics drivers.** The latest NVIDIA driver comes from the undocumented lookup behind NVIDIA's driver
  search. AMD and Intel offer no such lookup.
- **PC info** is read from the registry, the firmware's tables and Windows itself, in milliseconds and
  without admin rights.
- **Updates.** At most once a day the .exe asks GitHub for the latest release of `Zorakidd/UpdateCheck`
  (`GITHUB_REPO` in [updater.rs](src/updater.rs)). If its tag is a newer version and an .exe is attached, the
  app downloads it, checks its size and the SHA-256 digest GitHub publishes for it (a download without one
  isn't installed), swaps it in for the running .exe and restarts with the same arguments. Failures are
  logged and never stop the app; a repository without any release yet is no failure.

## Versions and releases

- **The version counts up by itself.** [build.rs](build.rs) takes the major and minor version from
  `Cargo.toml` and makes the number of commits the patch version, so each commit is one version higher than
  the one before, and a build of a commit has the same version on every PC. Raise the major or minor version
  in `Cargo.toml` only for a big change.
- **Every push to `main` that changes the app is released.** The [release workflow](.github/workflows/release.yml)
  runs the tests, builds the .exe and publishes it as the release `v<version>`, which the copies people run
  install within a day. A push that changes only documentation or notes releases nothing.
- The repository has to stay public, since the updater doesn't authenticate, and the README's download
  button points at the latest release's `UpdateCheck.exe`.
- `cargo build --release` builds the same .exe locally, at `target/release/UpdateCheck.exe`.
