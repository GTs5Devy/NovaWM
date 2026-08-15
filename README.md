<div align="center">

# NovaWM

**NovaWM is a Windows tiling window manager focused on synchronized
multi-monitor workspaces, interactive split/stack tiling, and persistent
layouts.**

Based on [GlazeWM](https://github.com/glzr-io/glazewm) and distributed under
GPL-3.0.

[Features](#features) |
[Installation](#installation) |
[Configuration](#configuration) |
[Development](#development) |
[License](#license)

</div>

## Features

- Synchronized workspaces across multiple monitors
- Independent workspace instances per monitor
- `alt+1` through `alt+4` synchronized workspace switching
- Same-monitor workspace moves with `move --workspace`
- Recursive horizontal and vertical tiling
- Interactive drag/drop split insertion
- Center-drop tab stacking
- Native `StackContainer` support
- Next/previous tab switching
- `unstack` support
- Dynamic resize/reflow with no dead gaps
- Persistent window monitor/workspace restoration
- Monitor reconnect restoration
- Native Alt+Tab compatibility
- YASB compatibility through GlazeWM-compatible IPC behavior
- YAML configuration

## Planned

- Visual drag/drop zones
- Launcher / command palette
- Themes / appearance customization
- Optional animations
- Richer stack UI

## Why NovaWM

NovaWM is aimed at multi-monitor Windows setups where each workspace is a
logical workstation across displays.

```text
Workspace 1
  Monitor 1: VS Code
  Monitor 2: browser/docs

Workspace 2
  Monitor 1: Roblox Studio
  Monitor 2: browser/docs
```

Switching to workspace 2 changes both monitors together while preserving each
monitor's own windows and layout.

## Installation

Download the latest Windows build from
[GitHub Releases](https://github.com/GTs5Devy/NovaWM/releases), extract the
ZIP, and run NovaWM.

Start NovaWM from the extracted folder:

```powershell
.\novawm.exe start --config=".\config.yaml"
```

Exit safely:

```powershell
.\novawm-cli.exe command wm-exit
```

The release ZIP includes:

```text
novawm.exe
novawm-cli.exe
novawm-watcher.exe
config.yaml
LICENSE.md
NOTICE.md
README.md
```

## Building From Source

Requirements:

- Windows 11
- Rust toolchain
- Git

Build from source:

```powershell
git clone https://github.com/GTs5Devy/NovaWM.git
cd NovaWM
cargo build --release --workspace
```

Main binaries:

```text
target\release\novawm.exe
target\release\novawm-cli.exe
target\release\novawm-watcher.exe
```

Start a source build with the included sample config:

```powershell
.\target\release\novawm.exe start --config=".\resources\assets\sample-config.yaml"
```

Exit safely:

```powershell
.\target\release\novawm-cli.exe command wm-exit
```

## Configuration

By default, NovaWM reads:

```text
%USERPROFILE%\.novawm\config.yaml
```

You can override the config path with:

```powershell
setx NOVAWM_CONFIG_PATH "C:\path\to\config.yaml"
```

`GLAZEWM_CONFIG_PATH` remains accepted as a compatibility fallback.

Small synchronized workspace sample:

```yaml
keybindings:
  - commands: ['focus-all-workspaces 1']
    bindings: ['alt+1']

  - commands: ['focus-all-workspaces 2']
    bindings: ['alt+2']

  - commands: ['move --workspace 1', 'focus-all-workspaces 1']
    bindings: ['alt+shift+1']

  - commands: ['focus-next-tab']
    bindings: ['alt+oem_close_brackets']

  - commands: ['focus-prev-tab']
    bindings: ['alt+oem_open_brackets']

  - commands: ['unstack']
    bindings: ['alt+shift+s']
```

## Smart Tiling And Stacks

NovaWM extends the existing recursive split tree with interactive drop
semantics:

```text
edge drop   -> split
center drop -> stack
```

Example:

```text
+---------------------+------------------+
|     PowerShell      |                  |
+---------------------+     Explorer     |
| Chrome | VS Code    |                  |
+---------------------+------------------+
```

A stack occupies one tile in the parent layout. Only the selected stack child
is visible, while hidden stack children remain managed on the same monitor and
workspace. Nested split ratios resize recursively so available space stays
filled.

## Multi-Monitor Workspaces

Workspace names may exist independently on each monitor:

```text
Monitor 1
  Workspace 1
  Workspace 2

Monitor 2
  Workspace 1
  Workspace 2
```

`focus-all-workspaces 2` activates each monitor's own workspace 2. This is one
of NovaWM's main differences from GlazeWM's original global workspace-name
model.

## Persistence

NovaWM stores v1 session state at:

```text
%USERPROFILE%\.novawm\novawm-session-v1.json
```

It restores window monitor/workspace placement across restarts and monitor
reconnects using process path, window class, process name, and normalized title
matching. HWND values are runtime-only and are not persisted as identity.

## YASB

NovaWM currently preserves GlazeWM-compatible IPC behavior so YASB's existing
GlazeWM workspace widget can continue to work. There is not yet a separate
native NovaWM YASB provider.

## Development

Useful checks:

```powershell
cargo fmt
cargo check
cargo test -p wm --bin novawm
cargo build --release --workspace
```

Current validation:

- 27 NovaWM WM tests passing
- `cargo test --workspace` may fail in `wm-platform` display tests when the
  test environment exposes zero displays

## Project Status

NovaWM v0.1.0 is an early public release. APIs, config shape, and runtime
behavior may still change.

## Based On GlazeWM

NovaWM is a modified work based on GlazeWM by the GlazeWM contributors.

NovaWM retains substantial portions of the original GlazeWM codebase and
extends it with its own multi-monitor workspace model, persistence system, and
interactive split/stack tiling behavior.

GlazeWM:

<https://github.com/glzr-io/glazewm>

NovaWM modifications began in 2026. NovaWM is distributed under GPL-3.0 and is
not affiliated with or endorsed by the GlazeWM maintainers.

See [NOTICE.md](NOTICE.md) and [LICENSE.md](LICENSE.md).

## License

NovaWM is distributed under GPL-3.0. See [LICENSE.md](LICENSE.md).
