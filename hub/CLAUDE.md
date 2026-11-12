# HalpTask Hub: notes for agents

## Keep `hub/target` small

`hub/target` once grew to 75 GB. These settings keep it down; don't undo them.

- **One shared build folder.** `app/src-tauri` is its own Cargo workspace but
  builds into `hub/target` through `app/src-tauri/.cargo/config.toml`. Don't
  add another `target-dir`, `CARGO_TARGET_DIR` or a second `target/` folder.
  Release bundles land in `hub/target/release/bundle/`.
- **Matching dev profiles.** `[profile.dev]` and `[profile.dev.package."*"]`
  must stay identical in `hub/Cargo.toml` and `app/src-tauri/Cargo.toml`, or
  the two workspaces stop reusing each other's dependency builds.
- **Slim debug info.** Dev builds use `debug = "line-tables-only"` for our
  crates and `debug = false` for dependencies. When you need a debugger with
  variable values, set `CARGO_PROFILE_DEV_DEBUG=true` for that build instead
  of changing the profile.
- **Trim when it grows.** Run `scripts/trim-target.sh` (from `hub/`) after
  long sessions, toolchain updates or dependency bumps. It removes
  incremental caches, then the whole folder, if `hub/target` is over 15 GB.
  Deleting `hub/target` is always safe; it's gitignored and only costs a
  full rebuild.
- Avoid builds that multiply artifacts without need: new feature
  combinations, `--release` builds just to check something, or extra
  toolchains. Prefer `cargo check` / `cargo clippy` when a full build isn't
  needed.

## Build and test

See the "Build and test" and "Disk space" sections in `README.md`.
