use anyhow::Context;
use tracing::info;
use wm_common::VecDequeExt;

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
      .queue_container_to_redraw(displayed_workspace)
      .queue_container_to_redraw(target_workspace);

    // Get empty workspace to destroy (if one is found). Cannot destroy
    // empty workspaces if they're the only workspace on the monitor.
    let workspace_to_destroy =
      state.workspaces().into_iter().find(|workspace| {
        !workspace.config().keep_alive
          && !workspace.has_children()
          && !workspace.is_displayed()
      });

    if let Some(workspace) = workspace_to_destroy {
      deactivate_workspace(workspace, state)?;
    }

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
  _config: &UserConfig,
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
      state.workspace_by_name_in_monitor(&monitor, workspace_name);

    let Some(target_workspace) = target_workspace else {
      continue;
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
        .queue_container_to_redraw(displayed_workspace);
    }

    state
      .pending_sync
      .queue_container_to_redraw(target_workspace.clone())
      .queue_workspace_to_reorder(target_workspace);

    switched_any = true;
  }

  if switched_any {
    let workspace_to_destroy =
      state.workspaces().into_iter().find(|workspace| {
        !workspace.config().keep_alive
          && !workspace.has_children()
          && !workspace.is_displayed()
      });

    if let Some(workspace) = workspace_to_destroy {
      deactivate_workspace(workspace, state)?;
    }
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
