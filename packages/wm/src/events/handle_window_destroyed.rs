use tracing::info;
use wm_platform::WindowId;

use crate::{
  commands::{
    window::unmanage_window, workspace::deactivate_empty_dynamic_workspace,
  },
  traits::WindowGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn handle_window_destroyed(
  native_window_id: WindowId,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let found_window = state
    .windows()
    .into_iter()
    .find(|window| window.native().id() == native_window_id);

  // Unmanage the window if it's currently managed.
  if let Some(window) = found_window {
    info!("Window closed: {window}");
    unmanage_window(window, state)?;

    // Destroy dynamic parent workspace if window was killed while its
    // workspace was not displayed (e.g. via task manager).
    deactivate_empty_dynamic_workspace(state, config)?;
  }

  Ok(())
}
