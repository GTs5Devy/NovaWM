# Contributing To NovaWM

Thanks for helping improve NovaWM.

NovaWM is a GPL-3.0 project derived from GlazeWM. Please keep changes scoped,
preserve upstream attribution, and avoid unrelated rewrites when working on a
feature or bug fix.

## Development

Build the workspace:

```sh
cargo build --workspace
```

Run the main checks:

```sh
cargo fmt
cargo check
cargo test --workspace
```

Run the release build:

```sh
cargo build --release --workspace
```

## Architecture Notes

The Rust crates intentionally still use compact internal names such as `wm`,
`wm-common`, `wm-platform`, `wm-cli`, and `wm-watcher`.

Important runtime binaries are:

- `novawm.exe`: main window manager
- `novawm-cli.exe`: CLI forwarder/client
- `novawm-watcher.exe`: watcher process used on Windows to restore hidden
  windows if the WM exits unexpectedly

NovaWM uses a command/event architecture. Parsed command types live in
`packages/wm-common/src/app_command.rs`, dispatch is handled in
`packages/wm/src/wm.rs`, and most command implementations live under
`packages/wm/src/commands`.

Windows are organized in a recursive container tree with monitors, workspaces,
split containers, stack containers, and window containers.
