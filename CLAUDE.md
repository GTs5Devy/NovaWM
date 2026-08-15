# NovaWM Repository Notes

NovaWM is a Rust tiling window manager for Windows derived from GlazeWM.

Workspace crates:

- `wm`: main application and window-management state machine
- `wm-common`: shared commands, DTOs, IPC messages, and config schema
- `wm-platform`: platform abstractions and Windows/macOS implementations
- `wm-cli`: CLI wrapper/client
- `wm-ipc-client`: IPC client library
- `wm-watcher`: Windows watcher process for cleanup after unexpected exits
- `wm-macros`: shared macros

Expected Windows release binaries:

- `target\release\novawm.exe`
- `target\release\novawm-cli.exe`
- `target\release\novawm-watcher.exe`

Common checks:

```sh
cargo fmt
cargo check
cargo test --workspace
cargo build --release --workspace
```

Runtime config defaults to `%USERPROFILE%\.novawm\config.yaml`. NovaWM keeps
temporary compatibility import paths for old GlazeWM-derived config/session
files so existing local testers do not lose configuration.
