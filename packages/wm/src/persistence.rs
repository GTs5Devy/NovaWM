use std::{
  collections::HashSet,
  fs,
  path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use wm_common::{DisplayState, WindowState};
use wm_platform::Rect;

use crate::{
  commands::{
    container::move_container_within_tree, window::update_window_state,
    workspace::activate_workspace,
  },
  models::{
    Monitor, NativeMonitorProperties, NativeWindowProperties, Workspace,
  },
  traits::{CommonGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub const SESSION_VERSION: u32 = 1;

#[derive(Clone, Debug, Default)]
pub struct PersistenceState {
  session: Option<PersistedSession>,
  used_entries: HashSet<usize>,
}

impl PersistenceState {
  pub fn clear(&mut self) {
    self.session = None;
    self.used_entries.clear();
  }
}

#[derive(Clone, Debug)]
pub struct WindowRestore {
  pub workspace: Workspace,
  pub state: Option<WindowState>,
  pub floating_placement: Option<Rect>,
  pub has_custom_floating_placement: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSession {
  pub version: u32,
  pub windows: Vec<PersistedWindow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedWindow {
  pub identity: PersistedWindowIdentity,
  pub assignment: PersistedWindowAssignment,
  pub window_state: WindowState,
  pub floating_placement: Rect,
  pub has_custom_floating_placement: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedWindowIdentity {
  pub process_path: Option<String>,
  pub window_class: Option<String>,
  pub process_name: String,
  pub normalized_title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedWindowAssignment {
  pub monitor: PersistedMonitorIdentity,
  pub workspace_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedMonitorIdentity {
  pub device_name: String,
  pub device_path: Option<String>,
  pub hardware_id: Option<String>,
}

impl WmState {
  pub fn load_persistence(&mut self, config: &UserConfig) {
    if !config.value.persistence.enabled {
      self.persistence.clear();
      return;
    }

    match load_session(&state_file_path()) {
      Ok(session) => {
        self.persistence.session = session;
        self.persistence.used_entries.clear();
      }
      Err(err) => {
        tracing::warn!("Failed to load persistence state: {err}");
      }
    }
  }

  pub fn save_persistence(&self, config: &UserConfig) {
    if !config.value.persistence.enabled {
      return;
    }

    let session = capture_session(self);
    let path = state_file_path();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
      handle.spawn_blocking(move || {
        if let Err(err) = save_session(&path, &session) {
          tracing::warn!("Failed to save persistence state: {err}");
        }
      });
    } else if let Err(err) = save_session(&path, &session) {
      tracing::warn!("Failed to save persistence state: {err}");
    }
  }

  pub fn save_persistence_sync(&self, config: &UserConfig) {
    if !config.value.persistence.enabled {
      return;
    }

    let session = capture_session(self);
    if let Err(err) = save_session(&state_file_path(), &session) {
      tracing::warn!("Failed to save persistence state: {err}");
    }
  }

  pub fn persistence_restore_for_window(
    &mut self,
    native_properties: &NativeWindowProperties,
    config: &UserConfig,
  ) -> Option<WindowRestore> {
    if !config.value.persistence.enabled
      || !config.value.persistence.restore_on_startup
    {
      return None;
    }

    let session = self.persistence.session.as_ref()?;
    let match_index = best_match_index(
      native_properties,
      &session.windows,
      &self.persistence.used_entries,
    )?;

    let persisted = session.windows[match_index].clone();
    let monitor = monitor_by_identity(
      &self.monitors(),
      &persisted.assignment.monitor,
    )?;
    let workspace = self.workspace_for_restore(
      &monitor,
      &persisted.assignment.workspace_name,
      config,
    )?;

    self.persistence.used_entries.insert(match_index);

    Some(WindowRestore {
      workspace,
      state: safe_window_state(&persisted.window_state),
      floating_placement: Some(persisted.floating_placement),
      has_custom_floating_placement: persisted
        .has_custom_floating_placement,
    })
  }

  pub fn restore_persisted_windows_on_monitor_reconnect(
    &mut self,
    config: &UserConfig,
  ) -> anyhow::Result<()> {
    if !config.value.persistence.enabled
      || !config.value.persistence.restore_on_monitor_reconnect
    {
      return Ok(());
    }

    let Some(session) = self.persistence.session.clone() else {
      return Ok(());
    };

    let mut used_entries = HashSet::new();

    for window in self.windows() {
      if matches!(
        window.display_state(),
        DisplayState::Hiding | DisplayState::Showing
      ) {
        continue;
      }

      let Some(match_index) = best_match_index(
        &window.native_properties(),
        &session.windows,
        &used_entries,
      ) else {
        continue;
      };

      let persisted = &session.windows[match_index];
      let Some(target_monitor) = monitor_by_identity(
        &self.monitors(),
        &persisted.assignment.monitor,
      ) else {
        continue;
      };

      let Some(target_workspace) = self.workspace_for_restore(
        &target_monitor,
        &persisted.assignment.workspace_name,
        config,
      ) else {
        continue;
      };

      used_entries.insert(match_index);

      if let Some(restored_state) =
        safe_window_state(&persisted.window_state)
      {
        update_window_state(window.clone(), restored_state, self, config)?;
      }

      let window = self
        .container_by_id(window.id())
        .and_then(|container| container.as_window_container().ok())
        .context("Restored window disappeared from state.")?;

      if window.workspace().map(|workspace| workspace.id())
        != Some(target_workspace.id())
      {
        move_container_within_tree(
          &window.clone().into(),
          &target_workspace.clone().into(),
          target_workspace.child_count(),
          self,
        )?;
      }

      window.set_floating_placement(persisted.floating_placement.clone());
      window.set_has_custom_floating_placement(
        persisted.has_custom_floating_placement,
      );

      self
        .pending_sync
        .queue_container_to_redraw(window.clone())
        .queue_workspace_to_reorder(target_workspace);
    }

    Ok(())
  }

  fn workspace_for_restore(
    &mut self,
    monitor: &Monitor,
    workspace_name: &str,
    config: &UserConfig,
  ) -> Option<Workspace> {
    if let Some(workspace) =
      self.workspace_by_name_in_monitor(monitor, workspace_name)
    {
      return Some(workspace);
    }

    config.workspace_config_by_name(workspace_name)?;

    if let Err(err) = activate_workspace(
      Some(workspace_name),
      Some(monitor.clone()),
      self,
      config,
    ) {
      tracing::warn!(
        workspace_name,
        monitor = ?monitor.id(),
        "Failed to recreate persisted workspace: {err}"
      );
      return None;
    }

    self.workspace_by_name_in_monitor(monitor, workspace_name)
  }
}

pub fn state_file_path() -> PathBuf {
  home::home_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join(".novawm/novawm-session-v1.json")
}

fn old_state_file_path() -> PathBuf {
  home::home_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join(".glzr/glazewm/novawm-session-v1.json")
}

pub fn capture_session(state: &WmState) -> PersistedSession {
  PersistedSession {
    version: SESSION_VERSION,
    windows: state
      .windows()
      .into_iter()
      .filter_map(|window| {
        let workspace = window.workspace()?;
        let monitor = workspace.monitor()?;
        let monitor_properties = monitor.native_properties();
        let native_properties = window.native_properties();

        Some(PersistedWindow {
          identity: PersistedWindowIdentity {
            process_path: native_properties.process_path,
            #[cfg(target_os = "windows")]
            window_class: Some(native_properties.class_name),
            #[cfg(not(target_os = "windows"))]
            window_class: None,
            process_name: native_properties.process_name,
            normalized_title: normalize_title(&native_properties.title),
          },
          assignment: PersistedWindowAssignment {
            monitor: PersistedMonitorIdentity::from(&monitor_properties),
            workspace_name: workspace.config().name,
          },
          window_state: window.state(),
          floating_placement: window.floating_placement(),
          has_custom_floating_placement: window
            .has_custom_floating_placement(),
        })
      })
      .collect(),
  }
}

pub fn load_session(
  path: &Path,
) -> anyhow::Result<Option<PersistedSession>> {
  if !path.exists() {
    let old_path = old_state_file_path();
    if old_path.exists() {
      let parent = path.parent().context("Invalid persistence path.")?;
      fs::create_dir_all(parent)?;
      fs::copy(&old_path, path).with_context(|| {
        format!(
          "Unable to import persistence state from {} to {}.",
          old_path.display(),
          path.display()
        )
      })?;
    } else {
      return Ok(None);
    }
  }

  if !path.exists() {
    return Ok(None);
  }

  let session: PersistedSession = serde_json::from_str(
    &fs::read_to_string(path)
      .with_context(|| format!("Unable to read {}.", path.display()))?,
  )?;

  if session.version != SESSION_VERSION {
    tracing::info!(
      version = session.version,
      "Ignoring unsupported persistence state version."
    );
    return Ok(None);
  }

  Ok(Some(session))
}

pub fn save_session(
  path: &Path,
  session: &PersistedSession,
) -> anyhow::Result<()> {
  let parent = path.parent().context("Invalid persistence path.")?;
  fs::create_dir_all(parent)?;

  let temp_path = path.with_extension("json.tmp");
  fs::write(&temp_path, serde_json::to_string_pretty(session)?)?;
  fs::rename(temp_path, path)?;

  Ok(())
}

fn best_match_index(
  native_properties: &NativeWindowProperties,
  windows: &[PersistedWindow],
  used_entries: &HashSet<usize>,
) -> Option<usize> {
  let mut scored = windows
    .iter()
    .enumerate()
    .filter(|(index, _)| !used_entries.contains(index))
    .filter_map(|(index, persisted)| {
      match_score(native_properties, &persisted.identity)
        .map(|score| (index, score))
    })
    .collect::<Vec<_>>();

  scored.sort_by_key(|(_, score)| std::cmp::Reverse(*score));

  let (best_index, best_score) = scored.first().copied()?;
  if best_score < 60 {
    return None;
  }

  if scored.get(1).is_some_and(|(_, score)| *score == best_score) {
    return None;
  }

  Some(best_index)
}

fn match_score(
  native_properties: &NativeWindowProperties,
  persisted: &PersistedWindowIdentity,
) -> Option<u32> {
  let mut score = 0;

  if let Some(persisted_path) = &persisted.process_path {
    if native_properties
      .process_path
      .as_ref()
      .is_some_and(|path| path.eq_ignore_ascii_case(persisted_path))
    {
      score += 100;
    } else {
      return None;
    }
  }

  #[cfg(target_os = "windows")]
  {
    if let Some(persisted_class) = &persisted.window_class {
      if native_properties
        .class_name
        .eq_ignore_ascii_case(persisted_class)
      {
        score += 40;
      } else {
        return None;
      }
    }
  }

  if native_properties
    .process_name
    .eq_ignore_ascii_case(&persisted.process_name)
  {
    score += 20;
  } else {
    return None;
  }

  if normalize_title(&native_properties.title)
    .zip(persisted.normalized_title.as_ref())
    .is_some_and(|(current_title, persisted_title)| {
      current_title == *persisted_title
    })
  {
    score += 5;
  }

  Some(score)
}

fn monitor_by_identity(
  monitors: &[Monitor],
  identity: &PersistedMonitorIdentity,
) -> Option<Monitor> {
  if let Some(device_path) = &identity.device_path {
    if let Some(monitor) = monitors.iter().find(|monitor| {
      monitor.native_properties().device_path.as_ref() == Some(device_path)
    }) {
      return Some(monitor.clone());
    }
  }

  if let Some(hardware_id) = &identity.hardware_id {
    let matching = monitors
      .iter()
      .filter(|monitor| {
        monitor.native_properties().hardware_id.as_ref()
          == Some(hardware_id)
      })
      .collect::<Vec<_>>();

    if matching.len() == 1 {
      return Some(matching[0].clone());
    }
  }

  let by_name = monitors
    .iter()
    .filter(|monitor| {
      monitor.native_properties().device_name == identity.device_name
    })
    .collect::<Vec<_>>();

  (by_name.len() == 1).then(|| by_name[0].clone())
}

fn safe_window_state(state: &WindowState) -> Option<WindowState> {
  match state {
    WindowState::Tiling
    | WindowState::Floating(_)
    | WindowState::Fullscreen(_) => Some(state.clone()),
    WindowState::Minimized => None,
  }
}

fn normalize_title(title: &str) -> Option<String> {
  let normalized = title
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();

  (!normalized.is_empty()).then_some(normalized)
}

impl From<&NativeMonitorProperties> for PersistedMonitorIdentity {
  fn from(properties: &NativeMonitorProperties) -> Self {
    Self {
      device_name: properties.device_name.clone(),
      #[cfg(target_os = "windows")]
      device_path: properties.device_path.clone(),
      #[cfg(not(target_os = "windows"))]
      device_path: None,
      #[cfg(target_os = "windows")]
      hardware_id: properties.hardware_id.clone(),
      #[cfg(not(target_os = "windows"))]
      hardware_id: None,
    }
  }
}

#[cfg(test)]
mod tests {
  use std::collections::HashSet;

  use tokio::sync::mpsc;
  use wm_common::{
    FloatingStateConfig, ParsedConfig, WindowState, WorkspaceConfig,
  };
  use wm_platform::{Dispatcher, Rect};

  use super::{
    best_match_index, match_score, PersistedMonitorIdentity,
    PersistedSession, PersistedWindow, PersistedWindowAssignment,
    PersistedWindowIdentity,
  };
  use crate::{
    commands::{
      container::{attach_container, set_focused_descendant},
      monitor::ensure_workspaces_for_monitor,
    },
    models::{Monitor, NativeWindowProperties, TilingWindow},
    test_utils,
    traits::CommonGetters,
    user_config::UserConfig,
    wm_state::WmState,
  };

  fn persisted_window(
    process_path: Option<&str>,
    window_class: Option<&str>,
    process_name: &str,
    title: Option<&str>,
  ) -> PersistedWindow {
    PersistedWindow {
      identity: PersistedWindowIdentity {
        process_path: process_path.map(str::to_string),
        window_class: window_class.map(str::to_string),
        process_name: process_name.to_string(),
        normalized_title: title.map(str::to_string),
      },
      assignment: PersistedWindowAssignment {
        monitor: PersistedMonitorIdentity {
          device_name: "monitor".to_string(),
          device_path: Some("device-path".to_string()),
          hardware_id: Some("hardware-id".to_string()),
        },
        workspace_name: "1".to_string(),
      },
      window_state: WindowState::Floating(FloatingStateConfig::default()),
      floating_placement: Rect::from_xy(1, 2, 3, 4),
      has_custom_floating_placement: true,
    }
  }

  fn native_window(
    process_path: Option<&str>,
    process_name: &str,
    title: &str,
  ) -> NativeWindowProperties {
    NativeWindowProperties::mock()
      .maybe_process_path(process_path.map(str::to_string))
      .process_name(process_name.to_string())
      .title(title.to_string())
      .call()
  }

  fn test_config() -> UserConfig {
    UserConfig::mock(ParsedConfig {
      persistence: wm_common::PersistenceConfig {
        enabled: true,
        restore_on_startup: true,
        restore_on_monitor_reconnect: true,
      },
      workspaces: ["1", "2", "3", "4"]
        .into_iter()
        .map(|name| WorkspaceConfig {
          name: name.to_string(),
          display_name: None,
          bind_to_monitor: None,
          keep_alive: false,
        })
        .collect(),
      ..ParsedConfig::default()
    })
  }

  fn test_state() -> WmState {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();
    WmState::new(Dispatcher::mock(), event_tx, exit_tx)
  }

  fn add_monitor(
    state: &mut WmState,
    config: &UserConfig,
    device_name: &str,
    device_path: &str,
  ) -> anyhow::Result<Monitor> {
    let monitor =
      Monitor::mock().device_name(device_name.to_string()).call();
    monitor.set_native_properties(
      crate::models::NativeMonitorProperties {
        device_name: device_name.to_string(),
        bounds: test_utils::mock_bounds(),
        working_area: test_utils::mock_working_area(),
        dpi: test_utils::MOCK_DPI,
        scale_factor: test_utils::MOCK_SCALE_FACTOR,
        #[cfg(target_os = "windows")]
        handle: 0,
        #[cfg(target_os = "windows")]
        hardware_id: Some(format!("{device_name}-hardware")),
        #[cfg(target_os = "windows")]
        device_path: Some(device_path.to_string()),
        #[cfg(target_os = "macos")]
        device_uuid: device_path.to_string(),
      },
    );

    attach_container(
      &monitor.clone().into(),
      &state.root_container.clone().into(),
      None,
    )?;
    ensure_workspaces_for_monitor(&monitor, state, config)?;

    Ok(monitor)
  }

  fn persisted_session(
    monitor_name: &str,
    monitor_path: &str,
    workspace_name: &str,
    process_path: &str,
    process_name: &str,
    title: Option<&str>,
  ) -> PersistedSession {
    PersistedSession {
      version: super::SESSION_VERSION,
      windows: vec![PersistedWindow {
        identity: PersistedWindowIdentity {
          process_path: Some(process_path.to_string()),
          window_class: Some(String::new()),
          process_name: process_name.to_string(),
          normalized_title: title.map(str::to_string),
        },
        assignment: PersistedWindowAssignment {
          monitor: PersistedMonitorIdentity {
            device_name: monitor_name.to_string(),
            device_path: Some(monitor_path.to_string()),
            hardware_id: Some(format!("{monitor_name}-hardware")),
          },
          workspace_name: workspace_name.to_string(),
        },
        window_state: WindowState::Tiling,
        floating_placement: Rect::from_xy(10, 20, 800, 600),
        has_custom_floating_placement: true,
      }],
    }
  }

  #[test]
  fn title_is_only_a_weak_disambiguator() {
    let native = native_window(
      Some("C:\\Apps\\Chrome\\chrome.exe"),
      "chrome",
      "New tab",
    );

    let strong = persisted_window(
      Some("C:\\Apps\\Chrome\\chrome.exe"),
      Some(""),
      "chrome",
      Some("old title"),
    );

    assert!(
      match_score(&native, &strong.identity).is_some_and(|s| s >= 60)
    );
  }

  #[test]
  fn ambiguous_persistence_matches_are_skipped() {
    let native = native_window(None, "chrome", "changed title");
    let entries = vec![
      persisted_window(None, Some(""), "chrome", Some("first")),
      persisted_window(None, Some(""), "chrome", Some("second")),
    ];

    assert_eq!(best_match_index(&native, &entries, &HashSet::new()), None);
  }

  #[test]
  fn persistence_restores_window_to_same_monitor_and_workspace(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let _monitor_0 =
      add_monitor(&mut state, &config, "monitor-0", "path-0")?;
    let monitor_1 =
      add_monitor(&mut state, &config, "monitor-1", "path-1")?;

    state.persistence.session = Some(persisted_session(
      "monitor-1",
      "path-1",
      "3",
      "C:\\Apps\\Code\\Code.exe",
      "Code",
      Some("old title"),
    ));

    let native =
      native_window(Some("C:\\Apps\\Code\\Code.exe"), "Code", "new title");

    let restore = state
      .persistence_restore_for_window(&native, &config)
      .expect("window should restore");

    assert_eq!(restore.workspace.config().name, "3");
    assert_eq!(
      restore.workspace.monitor().expect("workspace monitor").id(),
      monitor_1.id()
    );
    assert_eq!(restore.state, Some(WindowState::Tiling));
    assert_eq!(
      restore.floating_placement,
      Some(Rect::from_xy(10, 20, 800, 600))
    );

    Ok(())
  }

  #[test]
  fn persistence_ignores_stale_window_entries() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    add_monitor(&mut state, &config, "monitor-0", "path-0")?;

    state.persistence.session = Some(persisted_session(
      "monitor-0",
      "path-0",
      "2",
      "C:\\Apps\\Code\\Code.exe",
      "Code",
      None,
    ));

    let native = native_window(
      Some("C:\\Apps\\Chrome\\chrome.exe"),
      "chrome",
      "Browser",
    );

    assert!(state
      .persistence_restore_for_window(&native, &config)
      .is_none());

    Ok(())
  }

  #[test]
  fn persistence_falls_back_when_monitor_missing() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    add_monitor(&mut state, &config, "monitor-0", "path-0")?;

    state.persistence.session = Some(persisted_session(
      "monitor-1",
      "path-1",
      "3",
      "C:\\Apps\\Code\\Code.exe",
      "Code",
      None,
    ));

    let native =
      native_window(Some("C:\\Apps\\Code\\Code.exe"), "Code", "new title");

    assert!(state
      .persistence_restore_for_window(&native, &config)
      .is_none());

    Ok(())
  }

  #[test]
  fn configured_workspace_instances_restore_independently_on_two_monitors(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor_0 =
      add_monitor(&mut state, &config, "monitor-0", "path-0")?;
    let monitor_1 =
      add_monitor(&mut state, &config, "monitor-1", "path-1")?;

    let monitor_0_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_0, "1")
      .expect("monitor 0 workspace 1");
    let monitor_1_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_1, "1")
      .expect("monitor 1 workspace 1");

    let vscode = TilingWindow::mock()
      .title("VS Code".to_string())
      .process_name("Code".to_string())
      .maybe_process_path(Some("C:\\Apps\\Code\\Code.exe".to_string()))
      .call();
    let chrome = TilingWindow::mock()
      .title("Chrome".to_string())
      .process_name("chrome".to_string())
      .maybe_process_path(Some("C:\\Apps\\Chrome\\chrome.exe".to_string()))
      .call();

    attach_container(
      &vscode.clone().into(),
      &monitor_0_workspace_1.into(),
      None,
    )?;
    attach_container(
      &chrome.clone().into(),
      &monitor_1_workspace_1.into(),
      None,
    )?;
    set_focused_descendant(&vscode.clone().into(), None);

    state.persistence.session = Some(PersistedSession {
      version: super::SESSION_VERSION,
      windows: vec![
        persisted_session(
          "monitor-0",
          "path-0",
          "2",
          "C:\\Apps\\Code\\Code.exe",
          "Code",
          None,
        )
        .windows
        .remove(0),
        persisted_session(
          "monitor-1",
          "path-1",
          "3",
          "C:\\Apps\\Chrome\\chrome.exe",
          "chrome",
          None,
        )
        .windows
        .remove(0),
      ],
    });

    state.restore_persisted_windows_on_monitor_reconnect(&config)?;

    let vscode_workspace = vscode.workspace().expect("VS Code workspace");
    let chrome_workspace = chrome.workspace().expect("Chrome workspace");

    assert_eq!(vscode_workspace.config().name, "2");
    assert_eq!(
      vscode_workspace.monitor().expect("VS Code monitor").id(),
      monitor_0.id()
    );
    assert_eq!(chrome_workspace.config().name, "3");
    assert_eq!(
      chrome_workspace.monitor().expect("Chrome monitor").id(),
      monitor_1.id()
    );

    Ok(())
  }
}
