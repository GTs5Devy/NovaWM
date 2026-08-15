use std::{iter, path::PathBuf};

use clap::{error::KindFormatter, Args, Parser, ValueEnum};
use serde::{Deserialize, Deserializer, Serialize};
use tracing::Level;
use uuid::Uuid;
use wm_platform::{Delta, Direction, LengthValue, OpacityValue};

use crate::TilingDirection;

const VERSION: &str = env!("VERSION_NUMBER");

#[derive(Clone, Debug, Parser)]
#[clap(name = "novawm", author, version = VERSION, about, long_about = None)]
pub enum AppCommand {
  /// Starts the window manager.
  Start {
    /// Custom path to user config file.
    ///
    /// The default path is `%userprofile%/.novawm/config.yaml`
    #[clap(short = 'c', long = "config", value_hint = clap::ValueHint::FilePath)]
    config_path: Option<PathBuf>,

    #[clap(flatten)]
    verbosity: Verbosity,
  },

  /// Retrieves and outputs a specific part of the window manager's state.
  ///
  /// Requires an already running instance of the window manager.
  #[clap(alias = "q")]
  Query {
    #[clap(subcommand)]
    command: QueryCommand,
  },

  /// Invokes a window manager command.
  ///
  /// Requires an already running instance of the window manager.
  #[clap(alias = "c")]
  Command {
    #[clap(long = "id")]
    subject_container_id: Option<Uuid>,

    #[clap(subcommand)]
    command: InvokeCommand,
  },

  /// Subscribes to one or more WM events (e.g. `window_close`), and
  /// continuously outputs the incoming events.
  ///
  /// Requires an already running instance of the window manager.
  Sub {
    /// WM event(s) to subscribe to.
    #[clap(short = 'e', long, value_enum, num_args = 1..)]
    events: Vec<SubscribableEvent>,
  },

  /// Unsubscribes from a prior event subscription.
  ///
  /// Requires an already running instance of the window manager.
  Unsub {
    /// Subscription ID to unsubscribe from.
    #[clap(long = "id")]
    subscription_id: Uuid,
  },
}

impl AppCommand {
  /// Parses `AppCommand` from command line arguments.
  ///
  /// Defaults to `AppCommand::Start` if no arguments are provided.
  #[must_use]
  pub fn parse_with_default(args: &Vec<String>) -> Self {
    if args.len() == 1 {
      AppCommand::Start {
        config_path: None,
        verbosity: Verbosity {
          verbose: false,
          quiet: false,
        },
      }
    } else {
      AppCommand::parse_from(args)
    }
  }
}

/// Verbosity flags to be used with `#[command(flatten)]`.
#[derive(Args, Clone, Debug)]
#[clap(about = None, long_about = None)]
pub struct Verbosity {
  /// Enables verbose logging.
  #[clap(short = 'v', long, action)]
  verbose: bool,

  /// Disables logging.
  #[clap(short = 'q', long, action, conflicts_with = "verbose")]
  quiet: bool,
}

impl Verbosity {
  /// Gets the log level based on the verbosity flags.
  #[must_use]
  pub fn level(&self) -> Level {
    match (self.verbose, self.quiet) {
      (true, _) => Level::DEBUG,
      (_, true) => Level::ERROR,
      _ => Level::INFO,
    }
  }
}

#[derive(Clone, Debug, Parser)]
pub enum QueryCommand {
  /// Outputs metadata about the application (e.g. version number).
  AppMetadata,
  /// Outputs the active binding modes.
  BindingModes,
  /// Outputs the focused container (either a window or an empty
  /// workspace).
  Focused,
  /// Outputs the tiling direction of the focused container.
  TilingDirection,
  /// Outputs all monitors.
  Monitors,
  /// Outputs all windows.
  Windows,
  /// Outputs all active workspaces.
  Workspaces,
  /// Outputs whether the window manager is paused.
  Paused,
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum SubscribableEvent {
  All,
  ApplicationExiting,
  BindingModesChanged,
  FocusChanged,
  FocusedContainerMoved,
  MonitorAdded,
  MonitorUpdated,
  MonitorRemoved,
  TilingDirectionChanged,
  UserConfigChanged,
  WindowManaged,
  WindowUnmanaged,
  WorkspaceActivated,
  WorkspaceDeactivated,
  WorkspaceUpdated,
  PauseChanged,
}

#[derive(Clone, Debug, Parser, PartialEq, Serialize)]
pub enum InvokeCommand {
  AdjustBorders(InvokeAdjustBordersCommand),
  Close,
  Focus(InvokeFocusCommand),
  FocusAllWorkspaces {
    #[clap(required = true, allow_hyphen_values = true)]
    workspace: String,
  },
  FocusNextTab,
  FocusPrevTab,
  Ignore,
  Move(InvokeMoveCommand),
  MoveWorkspace {
    #[clap(long)]
    direction: Direction,
  },
  Position(InvokePositionCommand),
  Resize(InvokeResizeCommand),
  UpdateWorkspaceConfig {
    #[clap(long, allow_hyphen_values = true)]
    workspace: Option<String>,
    #[clap(flatten)]
    new_config: InvokeUpdateWorkspaceConfig,
  },
  SetFloating {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    centered: Option<bool>,

    #[clap(long, allow_hyphen_values = true)]
    x_pos: Option<i32>,

    #[clap(long, allow_hyphen_values = true)]
    y_pos: Option<i32>,

    #[clap(long, allow_hyphen_values = true)]
    width: Option<LengthValue>,

    #[clap(long, allow_hyphen_values = true)]
    height: Option<LengthValue>,
  },
  SetFullscreen {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    maximized: Option<bool>,
  },
  SetMinimized,
  SetTiling,
  SetTitleBarVisibility {
    #[clap(required = true, value_enum)]
    visibility: TitleBarVisibility,
  },
  SetTransparency(SetTransparencyCommand),
  ShellExec {
    #[clap(long, action)]
    hide_window: bool,

    #[clap(required = true, trailing_var_arg = true)]
    command: Vec<String>,
  },
  // Reuse `InvokeResizeCommand` struct.
  Size(InvokeResizeCommand),
  ToggleFloating {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    centered: Option<bool>,
  },
  ToggleFullscreen {
    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    shown_on_top: Option<bool>,

    #[clap(long, default_missing_value = "true", require_equals = true, num_args = 0..=1)]
    maximized: Option<bool>,
  },
  ToggleMinimized,
  ToggleTiling,
  ToggleTilingDirection,
  Unstack,
  SetTilingDirection {
    #[clap(required = true)]
    tiling_direction: TilingDirection,
  },
  WmCycleFocus {
    #[clap(long, default_value_t = false)]
    omit_floating: bool,

    #[clap(long, default_value_t = false)]
    omit_fullscreen: bool,

    #[clap(long, default_value_t = true)]
    omit_minimized: bool,

    #[clap(long, default_value_t = false)]
    omit_tiling: bool,
  },
  WmDisableBindingMode {
    #[clap(long)]
    name: String,
  },
  WmEnableBindingMode {
    #[clap(long)]
    name: String,
  },
  WmExit,
  WmRedraw,
  WmReloadConfig,
  WmTogglePause,
}

impl<'de> Deserialize<'de> for InvokeCommand {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    // Clap expects an array of string slices where the first argument is
    // the binary name/path. When deserializing commands from the user
    // config, we therefore have to prepend an additional empty argument.
    let unparsed = String::deserialize(deserializer)?;
    let unparsed_split = iter::once("").chain(unparsed.split_whitespace());

    InvokeCommand::try_parse_from(unparsed_split).map_err(|err| {
      // Format the error message and remove the "error: " prefix.
      let err_msg = err.apply::<KindFormatter>().to_string();
      serde::de::Error::custom(err_msg.trim_start_matches("error: "))
    })
  }
}

#[derive(Clone, Debug, PartialEq, Serialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TitleBarVisibility {
  Shown,
  Hidden,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokeAdjustBordersCommand {
  #[clap(long, allow_hyphen_values = true)]
  pub top: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub right: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub bottom: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub left: Option<LengthValue>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = false)]
#[allow(clippy::struct_excessive_bools)]
pub struct InvokeFocusCommand {
  #[clap(long)]
  pub direction: Option<Direction>,

  #[clap(long)]
  pub container_id: Option<Uuid>,

  #[clap(long)]
  pub workspace_in_direction: Option<Direction>,

  #[clap(long)]
  pub workspace: Option<String>,

  #[clap(long)]
  pub monitor: Option<usize>,

  #[clap(long)]
  pub next_active_workspace: bool,

  #[clap(long)]
  pub prev_active_workspace: bool,

  #[clap(long)]
  pub next_workspace: bool,

  #[clap(long)]
  pub prev_workspace: bool,

  #[clap(long)]
  pub next_active_workspace_on_monitor: bool,

  #[clap(long)]
  pub prev_active_workspace_on_monitor: bool,

  #[clap(long)]
  pub recent_workspace: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = false)]
#[allow(clippy::struct_excessive_bools)]
pub struct InvokeMoveCommand {
  /// Direction to move the window.
  #[clap(long)]
  pub direction: Option<Direction>,

  /// Move window to workspace in specified direction.
  #[clap(long)]
  pub workspace_in_direction: Option<Direction>,

  /// Name of workspace to move the window.
  #[clap(long)]
  pub workspace: Option<String>,

  #[clap(long)]
  pub next_active_workspace: bool,

  #[clap(long)]
  pub prev_active_workspace: bool,

  #[clap(long)]
  pub next_workspace: bool,

  #[clap(long)]
  pub prev_workspace: bool,

  #[clap(long)]
  pub next_active_workspace_on_monitor: bool,

  #[clap(long)]
  pub prev_active_workspace_on_monitor: bool,

  #[clap(long)]
  pub recent_workspace: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokeResizeCommand {
  #[clap(long, allow_hyphen_values = true)]
  pub width: Option<LengthValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub height: Option<LengthValue>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct SetTransparencyCommand {
  #[clap(long)]
  pub opacity: Option<OpacityValue>,

  #[clap(long, allow_hyphen_values = true)]
  pub opacity_delta: Option<Delta<OpacityValue>>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokePositionCommand {
  #[clap(long, action)]
  pub centered: bool,

  #[clap(long, allow_hyphen_values = true)]
  pub x_pos: Option<i32>,

  #[clap(long, allow_hyphen_values = true)]
  pub y_pos: Option<i32>,
}

#[derive(Args, Clone, Debug, PartialEq, Serialize)]
#[group(required = true, multiple = true)]
pub struct InvokeUpdateWorkspaceConfig {
  #[clap(long, allow_hyphen_values = true)]
  pub name: Option<String>,

  #[clap(long, allow_hyphen_values = true)]
  pub display_name: Option<String>,

  #[clap(long)]
  pub bind_to_monitor: Option<u32>,

  #[clap(long)]
  pub keep_alive: Option<bool>,
}

#[cfg(test)]
mod tests {
  use clap::Parser;

  use super::{AppCommand, InvokeCommand};

  fn parse_invoke(command: &str) -> InvokeCommand {
    InvokeCommand::parse_from(
      std::iter::once("").chain(command.split_whitespace()),
    )
  }

  #[test]
  fn parses_every_public_invoke_command_family() {
    let commands = [
      "adjust-borders --top 1px",
      "close",
      "focus --direction left",
      "focus --container-id 00000000-0000-0000-0000-000000000000",
      "focus --workspace 1",
      "focus --monitor 0",
      "focus --next-active-workspace",
      "focus --prev-active-workspace",
      "focus --next-workspace",
      "focus --prev-workspace",
      "focus --next-active-workspace-on-monitor",
      "focus --prev-active-workspace-on-monitor",
      "focus --recent-workspace",
      "focus --workspace-in-direction right",
      "focus-all-workspaces 2",
      "focus-next-tab",
      "focus-prev-tab",
      "ignore",
      "move --direction left",
      "move --workspace 2",
      "move --workspace-in-direction right",
      "move --next-active-workspace",
      "move --prev-active-workspace",
      "move --next-workspace",
      "move --prev-workspace",
      "move --next-active-workspace-on-monitor",
      "move --prev-active-workspace-on-monitor",
      "move --recent-workspace",
      "move-workspace --direction right",
      "position --centered",
      "position --x-pos 10 --y-pos 20",
      "resize --width +2%",
      "resize --height -2%",
      "update-workspace-config --workspace 1 --name dev",
      "set-floating --centered",
      "set-fullscreen --maximized",
      "set-minimized",
      "set-tiling",
      "set-title-bar-visibility hidden",
      "set-transparency --opacity 95%",
      "shell-exec cmd",
      "size --width 800px --height 600px",
      "toggle-floating --centered",
      "toggle-fullscreen --maximized",
      "toggle-minimized",
      "toggle-tiling",
      "toggle-tiling-direction",
      "unstack",
      "set-tiling-direction vertical",
      "wm-cycle-focus",
      "wm-disable-binding-mode --name resize",
      "wm-enable-binding-mode --name resize",
      "wm-exit",
      "wm-redraw",
      "wm-reload-config",
      "wm-toggle-pause",
    ];

    for command in commands {
      parse_invoke(command);
    }
  }

  #[test]
  fn parses_combined_novawm_workspace_binding_commands() {
    assert_eq!(
      parse_invoke("move --workspace 2"),
      InvokeCommand::Move(super::InvokeMoveCommand {
        direction: None,
        workspace_in_direction: None,
        workspace: Some("2".to_string()),
        next_active_workspace: false,
        prev_active_workspace: false,
        next_workspace: false,
        prev_workspace: false,
        next_active_workspace_on_monitor: false,
        prev_active_workspace_on_monitor: false,
        recent_workspace: false,
      })
    );

    assert_eq!(
      parse_invoke("focus-all-workspaces 2"),
      InvokeCommand::FocusAllWorkspaces {
        workspace: "2".to_string()
      }
    );
  }

  #[test]
  fn parses_cli_command_forwarding_shapes() {
    AppCommand::parse_from(["novawm", "query", "windows"]);
    AppCommand::parse_from(["novawm", "q", "monitors"]);
    AppCommand::parse_from([
      "novawm",
      "command",
      "focus-all-workspaces",
      "1",
    ]);
    AppCommand::parse_from([
      "novawm",
      "command",
      "--id",
      "00000000-0000-0000-0000-000000000000",
      "focus-next-tab",
    ]);
    AppCommand::parse_from(["novawm", "sub", "-e", "workspace_updated"]);
    AppCommand::parse_from([
      "novawm",
      "unsub",
      "--id",
      "00000000-0000-0000-0000-000000000000",
    ]);
  }
}
