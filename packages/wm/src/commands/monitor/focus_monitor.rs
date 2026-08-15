use anyhow::Context;

use crate::{
  commands::container::set_focused_descendant, traits::CommonGetters,
  user_config::UserConfig, wm_state::WmState,
};

/// Focuses a monitor by a given monitor index.
pub fn focus_monitor(
  monitor_index: usize,
  state: &mut WmState,
  _config: &UserConfig,
) -> anyhow::Result<()> {
  let monitors = state.monitors();

  let target_monitor = monitors.get(monitor_index).with_context(|| {
    format!("Monitor at index {monitor_index} was not found.")
  })?;

  let workspace = target_monitor
    .displayed_workspace()
    .context("Failed to get target workspace.")?;

  let focused_descendant = workspace.descendant_focus_order().next();
  let container_to_focus =
    focused_descendant.unwrap_or_else(|| workspace.into());

  set_focused_descendant(&container_to_focus, None);
  state.pending_sync.queue_focus_change().queue_cursor_jump();

  Ok(())
}
