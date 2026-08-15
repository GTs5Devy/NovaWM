use anyhow::Context;
use tracing::{debug, info};
use wm_common::{VecDequeExt, WmEvent};

use super::activate_workspace;
use crate::{
  commands::{
    container::set_focused_descendant, workspace::deactivate_workspace,
  },
  models::WorkspaceTarget,
  traits::CommonGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

/// Focuses a workspace by a given target.
///
/// This target can be a workspace name, the most recently focused
/// workspace, the next workspace, the previous workspace, or the workspace
/// in a given direction from the currently focused workspace.
///
/// The workspace will be activated if it isn't already active.
pub fn focus_workspace(
  target: WorkspaceTarget,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let focused_workspace = state
    .focused_container()
    .and_then(|focused| focused.workspace())
    .context("No workspace is currently focused.")?;

  let focused_monitor =
    focused_workspace.monitor().context("No focused monitor.")?;

  let (target_workspace_name, target_workspace) =
    state.workspace_by_target(&focused_workspace, target, config)?;

  // Retrieve or activate the target workspace by its name.
  let target_workspace = match target_workspace {
    Some(_) => anyhow::Ok(target_workspace),
    _ => match target_workspace_name {
      Some(name) => {
        activate_workspace(
          Some(&name),
          Some(focused_monitor.clone()),
          state,
          config,
        )?;

        Ok(state.workspace_by_name_in_monitor(&focused_monitor, &name))
      }
      _ => Ok(None),
    },
  }?;

  if let Some(target_workspace) = target_workspace {
    info!("Focusing workspace: {target_workspace}");

    // Get the currently displayed workspace on the same monitor that the
    // workspace to focus is on.
    let displayed_workspace = target_workspace
      .monitor()
      .and_then(|monitor| monitor.displayed_workspace())
      .context("No workspace is currently displayed.")?;

    // Set focus to whichever window last had focus in workspace. If the
    // workspace has no windows, then set focus to the workspace itself.
    let container_to_focus = target_workspace
      .descendant_focus_order()
      .next()
      .unwrap_or_else(|| target_workspace.clone().into());

    set_focused_descendant(&container_to_focus, None);
    state.pending_sync.queue_focus_change();

    // Display the workspace to switch focus to.
    state
      .pending_sync
      .queue_container_to_redraw(displayed_workspace.clone())
      .queue_container_to_redraw(target_workspace.clone());

    state.emit_event(WmEvent::WorkspaceUpdated {
      updated_workspace: displayed_workspace.to_dto()?,
    });

    state.emit_event(WmEvent::WorkspaceUpdated {
      updated_workspace: target_workspace.to_dto()?,
    });

    deactivate_empty_dynamic_workspace(state, config)?;

    // Save the currently focused workspace as recent.
    state
      .recent_workspace_name_by_monitor
      .insert(focused_monitor.id(), focused_workspace.config().name);
    state.pending_sync.queue_cursor_jump();
  }

  Ok(())
}

/// Focuses the workspace with the given name on every monitor where that
/// workspace is already active.
pub fn focus_all_workspaces(
  workspace_name: &str,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let focused_container = state
    .focused_container()
    .context("No container is currently focused.")?;

  let focused_workspace = focused_container
    .workspace()
    .context("No workspace is currently focused.")?;

  let focused_monitor = focused_workspace
    .monitor()
    .context("No monitor is currently focused.")?;

  let mut switched_any = false;

  for monitor in state.monitors() {
    let target_workspace =
      match state.workspace_by_name_in_monitor(&monitor, workspace_name) {
        Some(workspace) => workspace,
        None
          if config.workspace_config_by_name(workspace_name).is_some() =>
        {
          activate_workspace(
            Some(workspace_name),
            Some(monitor.clone()),
            state,
            config,
          )?;

          state
            .workspace_by_name_in_monitor(&monitor, workspace_name)
            .context("Failed to activate workspace on monitor.")?
        }
        None => continue,
      };

    let displayed_workspace = monitor.displayed_workspace();

    if displayed_workspace
      .as_ref()
      .is_some_and(|workspace| workspace.id() == target_workspace.id())
    {
      continue;
    }

    info!("Focusing workspace on monitor: {target_workspace}");

    monitor
      .borrow_child_focus_order_mut()
      .shift_to_index(0, target_workspace.id());

    if let Some(displayed_workspace) = displayed_workspace {
      state
        .recent_workspace_name_by_monitor
        .insert(monitor.id(), displayed_workspace.config().name);

      state
        .pending_sync
        .queue_container_to_redraw(displayed_workspace.clone());

      state.emit_event(WmEvent::WorkspaceUpdated {
        updated_workspace: displayed_workspace.to_dto()?,
      });
    }

    state
      .pending_sync
      .queue_container_to_redraw(target_workspace.clone())
      .queue_workspace_to_reorder(target_workspace.clone());

    state.emit_event(WmEvent::WorkspaceUpdated {
      updated_workspace: target_workspace.to_dto()?,
    });

    switched_any = true;
  }

  if switched_any {
    deactivate_empty_dynamic_workspace(state, config)?;
  }

  let container_to_focus = focused_monitor
    .workspaces()
    .into_iter()
    .find(|workspace| workspace.config().name == workspace_name)
    .filter(|workspace| workspace.is_displayed())
    .map(|workspace| {
      let focused_descendant = workspace.descendant_focus_order().next();
      focused_descendant.unwrap_or_else(|| workspace.into())
    })
    .unwrap_or(focused_container);

  set_focused_descendant(&container_to_focus, None);
  state.pending_sync.queue_focus_change().queue_cursor_jump();

  Ok(())
}

pub fn deactivate_empty_dynamic_workspace(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let workspace_to_destroy =
    state.workspaces().into_iter().find(|workspace| {
      let workspace_config = workspace.config();
      let is_configured = config
        .workspace_config_by_name(&workspace_config.name)
        .is_some();
      let child_count = workspace.child_count();
      let is_displayed = workspace.is_displayed();

      debug!(
        monitor = ?workspace.monitor().map(|monitor| monitor.id()),
        workspace_name = workspace_config.name,
        workspace_id = ?workspace.id(),
        is_displayed,
        child_count,
        keep_alive = workspace_config.keep_alive,
        is_configured,
        "Evaluating empty workspace cleanup."
      );

      !is_configured
        && !workspace_config.keep_alive
        && child_count == 0
        && !is_displayed
    });

  if let Some(workspace) = workspace_to_destroy {
    deactivate_workspace(workspace, state)?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use anyhow::Context;
  use tokio::sync::mpsc;
  use wm_common::{ContainerDto, ParsedConfig, WmEvent, WorkspaceConfig};
  use wm_platform::{Dispatcher, Rect};

  use super::{focus_all_workspaces, focus_workspace};
  use crate::{
    commands::{
      container::{attach_container, set_focused_descendant},
      monitor::ensure_workspaces_for_monitor,
      window::move_window_to_workspace,
      workspace::deactivate_workspace,
    },
    models::{Monitor, TilingWindow, WorkspaceTarget},
    test_utils,
    traits::{CommonGetters, PositionGetters},
    user_config::UserConfig,
    wm_state::WmState,
  };

  fn test_config() -> UserConfig {
    UserConfig::mock(ParsedConfig {
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

  fn test_state_with_events() -> (WmState, mpsc::UnboundedReceiver<WmEvent>)
  {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();

    (
      WmState::new(Dispatcher::mock(), event_tx, exit_tx),
      event_rx,
    )
  }

  fn test_state() -> WmState {
    test_state_with_events().0
  }

  fn add_monitor(
    state: &mut WmState,
    config: &UserConfig,
    device_name: &str,
  ) -> anyhow::Result<Monitor> {
    add_monitor_with_bounds(
      state,
      config,
      device_name,
      test_utils::mock_bounds(),
      test_utils::mock_working_area(),
    )
  }

  fn add_monitor_with_bounds(
    state: &mut WmState,
    config: &UserConfig,
    device_name: &str,
    bounds: Rect,
    working_area: Rect,
  ) -> anyhow::Result<Monitor> {
    let monitor = Monitor::mock()
      .device_name(device_name.to_string())
      .bounds(bounds)
      .working_area(working_area)
      .call();

    attach_container(
      &monitor.clone().into(),
      &state.root_container.clone().into(),
      None,
    )?;

    ensure_workspaces_for_monitor(&monitor, state, config)?;

    Ok(monitor)
  }

  fn workspace_names(monitor: &Monitor) -> Vec<String> {
    let mut names = monitor
      .workspaces()
      .into_iter()
      .map(|workspace| workspace.config().name)
      .collect::<Vec<_>>();

    names.sort();
    names
  }

  fn workspace_dtos(
    monitor: &Monitor,
  ) -> anyhow::Result<Vec<wm_common::WorkspaceDto>> {
    let ContainerDto::Monitor(monitor_dto) = monitor.to_dto()? else {
      unreachable!("Monitor should serialize to monitor DTO.");
    };

    Ok(
      monitor_dto
        .children
        .into_iter()
        .filter_map(|child| match child {
          ContainerDto::Workspace(workspace) => Some(workspace),
          _ => None,
        })
        .collect(),
    )
  }

  #[test]
  fn configured_empty_workspace_survives_switching_away_and_back(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config, "monitor-0")?;

    let workspace_1 = state
      .workspace_by_name_in_monitor(&monitor, "1")
      .expect("workspace 1 should exist");
    set_focused_descendant(&workspace_1.into(), None);

    for name in ["2", "3", "4", "1", "2", "3", "4"] {
      focus_workspace(
        WorkspaceTarget::Name(name.to_string()),
        &mut state,
        &config,
      )?;

      assert_eq!(workspace_names(&monitor), ["1", "2", "3", "4"]);
      assert_eq!(
        monitor
          .displayed_workspace()
          .expect("monitor should have displayed workspace")
          .config()
          .name,
        name
      );
    }

    Ok(())
  }

  #[test]
  fn configured_workspace_instances_survive_independently_on_two_monitors(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor_0 = add_monitor(&mut state, &config, "monitor-0")?;
    let monitor_1 = add_monitor(&mut state, &config, "monitor-1")?;

    let monitor_0_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_0, "1")
      .expect("monitor 0 workspace 1 should exist");
    let monitor_1_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_1, "1")
      .expect("monitor 1 workspace 1 should exist");

    assert_ne!(monitor_0_workspace_1.id(), monitor_1_workspace_1.id());

    set_focused_descendant(&monitor_0_workspace_1.into(), None);

    for name in ["2", "3", "4", "1", "2", "3", "4"] {
      focus_all_workspaces(name, &mut state, &config)?;

      assert_eq!(workspace_names(&monitor_0), ["1", "2", "3", "4"]);
      assert_eq!(workspace_names(&monitor_1), ["1", "2", "3", "4"]);
      assert_eq!(
        monitor_0
          .displayed_workspace()
          .expect("monitor 0 should have displayed workspace")
          .config()
          .name,
        name
      );
      assert_eq!(
        monitor_1
          .displayed_workspace()
          .expect("monitor 1 should have displayed workspace")
          .config()
          .name,
        name
      );
    }

    let monitor_1_workspace_3 = state
      .workspace_by_name_in_monitor(&monitor_1, "3")
      .expect("monitor 1 workspace 3 should exist");
    deactivate_workspace(monitor_1_workspace_3, &state)?;

    assert!(state
      .workspace_by_name_in_monitor(&monitor_1, "3")
      .is_none());

    focus_all_workspaces("3", &mut state, &config)?;

    assert!(state
      .workspace_by_name_in_monitor(&monitor_1, "3")
      .is_some());
    assert_eq!(workspace_names(&monitor_0), ["1", "2", "3", "4"]);
    assert_eq!(workspace_names(&monitor_1), ["1", "2", "3", "4"]);

    Ok(())
  }

  #[test]
  fn focus_all_workspaces_emits_workspace_updates_and_serializes_displayed_workspaces(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let (mut state, mut event_rx) = test_state_with_events();
    let monitor_0 = add_monitor(&mut state, &config, "monitor-0")?;
    let monitor_1 = add_monitor(&mut state, &config, "monitor-1")?;

    let workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_0, "1")
      .expect("monitor 0 workspace 1 should exist");
    set_focused_descendant(&workspace_1.into(), None);
    state.mark_initialized_for_test();

    focus_all_workspaces("2", &mut state, &config)?;

    let mut workspace_updates = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
      if let WmEvent::WorkspaceUpdated {
        updated_workspace: ContainerDto::Workspace(workspace),
      } = event
      {
        workspace_updates.push(workspace);
      }
    }

    assert_eq!(
      workspace_updates
        .iter()
        .filter(
          |workspace| workspace.name == "1" && !workspace.is_displayed
        )
        .count(),
      2
    );
    assert_eq!(
      workspace_updates
        .iter()
        .filter(|workspace| workspace.name == "2" && workspace.is_displayed)
        .count(),
      2
    );

    for monitor in [&monitor_0, &monitor_1] {
      let workspaces = workspace_dtos(monitor)?;

      assert!(
        workspaces.iter().any(
          |workspace| workspace.name == "1" && !workspace.is_displayed
        )
      );
      assert!(workspaces
        .iter()
        .any(|workspace| workspace.name == "2" && workspace.is_displayed));
    }

    let focused_workspaces = [&monitor_0, &monitor_1]
      .into_iter()
      .map(workspace_dtos)
      .collect::<anyhow::Result<Vec<_>>>()?
      .into_iter()
      .flatten()
      .filter(|workspace| workspace.has_focus)
      .collect::<Vec<_>>();

    assert_eq!(focused_workspaces.len(), 1);
    assert_eq!(focused_workspaces[0].name, "2");

    Ok(())
  }

  #[test]
  fn windows_preserve_monitor_ownership_across_workspace_switch(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor_0 = add_monitor_with_bounds(
      &mut state,
      &config,
      "monitor-0",
      Rect::from_xy(0, 0, 1680, 1050),
      Rect::from_xy(0, 0, 1680, 1000),
    )?;
    let monitor_1 = add_monitor_with_bounds(
      &mut state,
      &config,
      "monitor-1",
      Rect::from_xy(1680, 0, 1680, 1050),
      Rect::from_xy(1680, 0, 1680, 1000),
    )?;

    let monitor_0_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_0, "1")
      .expect("monitor 0 workspace 1 should exist");
    let monitor_1_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_1, "1")
      .expect("monitor 1 workspace 1 should exist");

    let powershell = TilingWindow::mock()
      .title("PowerShell".to_string())
      .process_name("powershell".to_string())
      .call();
    let chrome = TilingWindow::mock()
      .title("Chrome".to_string())
      .process_name("chrome".to_string())
      .call();

    attach_container(
      &powershell.clone().into(),
      &monitor_0_workspace_1.clone().into(),
      None,
    )?;
    attach_container(
      &chrome.clone().into(),
      &monitor_1_workspace_1.clone().into(),
      None,
    )?;

    set_focused_descendant(&powershell.clone().into(), None);

    for name in ["2", "1", "2", "1"] {
      focus_all_workspaces(name, &mut state, &config)?;
    }

    let chrome_workspace = chrome.workspace().context("No workspace.")?;
    let chrome_monitor = chrome.monitor().context("No monitor.")?;
    let chrome_rect = chrome.to_rect()?;

    assert_eq!(chrome_workspace.id(), monitor_1_workspace_1.id());
    assert_eq!(chrome_monitor.id(), monitor_1.id());
    assert!(monitor_1
      .native_properties()
      .working_area
      .contains_rect(&chrome_rect));
    assert!(!monitor_0
      .native_properties()
      .working_area
      .contains_rect(&chrome_rect));

    Ok(())
  }

  #[test]
  fn move_window_to_workspace_resolves_target_on_same_monitor(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor_0 = add_monitor_with_bounds(
      &mut state,
      &config,
      "monitor-0",
      Rect::from_xy(0, 0, 1680, 1050),
      Rect::from_xy(0, 0, 1680, 1000),
    )?;
    let monitor_1 = add_monitor_with_bounds(
      &mut state,
      &config,
      "monitor-1",
      Rect::from_xy(1680, 0, 1680, 1050),
      Rect::from_xy(1680, 0, 1680, 1000),
    )?;

    let monitor_0_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_0, "1")
      .expect("monitor 0 workspace 1 should exist");
    let monitor_1_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_1, "1")
      .expect("monitor 1 workspace 1 should exist");
    let monitor_1_workspace_3 = state
      .workspace_by_name_in_monitor(&monitor_1, "3")
      .expect("monitor 1 workspace 3 should exist");

    let vscode = TilingWindow::mock()
      .title("VS Code".to_string())
      .process_name("Code".to_string())
      .call();
    let chrome = TilingWindow::mock()
      .title("Chrome".to_string())
      .process_name("chrome".to_string())
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

    set_focused_descendant(&chrome.clone().into(), None);

    move_window_to_workspace(
      chrome.clone().into(),
      WorkspaceTarget::Name("3".to_string()),
      &mut state,
      &config,
    )?;

    let chrome_workspace = chrome.workspace().context("No workspace.")?;
    let chrome_monitor = chrome.monitor().context("No monitor.")?;

    assert_eq!(chrome_workspace.id(), monitor_1_workspace_3.id());
    assert_eq!(chrome_monitor.id(), monitor_1.id());
    assert_eq!(chrome_workspace.config().name, "3");
    assert!(state
      .workspace_by_name_in_monitor(&monitor_0, "3")
      .expect("monitor 0 workspace 3 should still exist")
      .descendants()
      .all(|container| container.id() != chrome.id()));

    Ok(())
  }

  #[test]
  fn move_then_focus_all_preserves_monitor_ownership() -> anyhow::Result<()>
  {
    let config = test_config();
    let mut state = test_state();
    let monitor_0 = add_monitor_with_bounds(
      &mut state,
      &config,
      "monitor-0",
      Rect::from_xy(0, 0, 1680, 1050),
      Rect::from_xy(0, 0, 1680, 1000),
    )?;
    let monitor_1 = add_monitor_with_bounds(
      &mut state,
      &config,
      "monitor-1",
      Rect::from_xy(1680, 0, 1680, 1050),
      Rect::from_xy(1680, 0, 1680, 1000),
    )?;

    let monitor_0_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_0, "1")
      .expect("monitor 0 workspace 1 should exist");
    let monitor_1_workspace_1 = state
      .workspace_by_name_in_monitor(&monitor_1, "1")
      .expect("monitor 1 workspace 1 should exist");

    let vscode = TilingWindow::mock()
      .title("VS Code".to_string())
      .process_name("Code".to_string())
      .call();
    let chrome = TilingWindow::mock()
      .title("Chrome".to_string())
      .process_name("chrome".to_string())
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

    set_focused_descendant(&chrome.clone().into(), None);

    for name in ["2", "3", "4", "1", "3"] {
      move_window_to_workspace(
        chrome.clone().into(),
        WorkspaceTarget::Name(name.to_string()),
        &mut state,
        &config,
      )?;
      focus_all_workspaces(name, &mut state, &config)?;

      let chrome_workspace =
        chrome.workspace().context("No workspace.")?;
      let chrome_monitor = chrome.monitor().context("No monitor.")?;
      let chrome_rect = chrome.to_rect()?;

      assert_eq!(chrome_workspace.config().name, name);
      assert_eq!(chrome_monitor.id(), monitor_1.id());
      assert!(monitor_1
        .native_properties()
        .working_area
        .contains_rect(&chrome_rect));
      assert_eq!(
        monitor_1
          .displayed_workspace()
          .expect("monitor 1 should have displayed workspace")
          .config()
          .name,
        name
      );
    }

    Ok(())
  }
}
