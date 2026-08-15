use anyhow::Context;
use wm_common::WmEvent;
use wm_platform::Direction;

use super::{
  activate_workspace, deactivate_empty_dynamic_workspace, sort_workspaces,
};
use crate::{
  commands::container::move_container_within_tree,
  models::Workspace,
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn move_workspace_in_direction(
  workspace: &Workspace,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let origin_monitor = workspace.monitor().context("No monitor.")?;
  let target_monitor =
    state.monitor_in_direction(&origin_monitor, direction)?;

  if let Some(target_monitor) = target_monitor {
    if target_monitor
      .workspaces()
      .into_iter()
      .any(|target_workspace| {
        target_workspace.config().name == workspace.config().name
      })
    {
      anyhow::bail!(
        "Target monitor already has workspace '{}'.",
        workspace.config().name
      );
    }

    // Get currently displayed workspace on the target monitor.
    let displayed_workspace = target_monitor
      .displayed_workspace()
      .context("No displayed workspace.")?;

    move_container_within_tree(
      &workspace.clone().into(),
      &target_monitor.clone().into(),
      target_monitor.child_count(),
      state,
    )?;

    let windows = workspace
      .descendants()
      .filter_map(|descendant| descendant.as_window_container().ok());

    for window in windows {
      window.set_has_pending_dpi_adjustment(true);

      window.set_floating_placement(
        window
          .floating_placement()
          .translate_to_center(&workspace.to_rect()?),
      );
    }

    state
      .pending_sync
      .queue_cursor_jump()
      .queue_container_to_redraw(workspace.clone())
      .queue_container_to_redraw(displayed_workspace);

    match origin_monitor.child_count() {
      0 => {
        // Prevent origin monitor from having no workspaces.
        activate_workspace(None, Some(origin_monitor), state, config)?;
      }
      _ => {
        // Redraw the workspace on the origin monitor.
        state.pending_sync.queue_container_to_redraw(
          origin_monitor
            .displayed_workspace()
            .context("No displayed workspace.")?,
        );
      }
    }

    deactivate_empty_dynamic_workspace(state, config)?;

    sort_workspaces(&target_monitor, config)?;

    state.emit_event(WmEvent::WorkspaceUpdated {
      updated_workspace: workspace.to_dto()?,
    });
  }

  Ok(())
}
