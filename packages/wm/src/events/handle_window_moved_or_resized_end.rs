use anyhow::Context;
use wm_common::{
  try_warn, FullscreenStateConfig, TilingDirection, WindowState,
};
use wm_platform::{LengthValue, Point, Rect};

use crate::{
  commands::{
    container::{
      move_container_within_tree, set_focused_descendant,
      wrap_in_split_container, wrap_in_stack_container,
    },
    window::{set_window_size, update_window_state},
  },
  events::update_floating_window_position,
  models::{
    DirectionContainer, NonTilingWindow, SplitContainer, StackContainer,
    TilingContainer, WindowContainer,
  },
  traits::{
    CommonGetters, PositionGetters, TilingDirectionGetters, WindowGetters,
  },
  user_config::UserConfig,
  wm_state::WmState,
};

/// Handles the event for when a window is finished being moved or resized
/// by the user (e.g. via the window's drag handles).
///
/// This resizes the window if it's a tiling window and attach a dragged
/// floating window.
///
/// TODO: Move this to a better location - maybe a new `active_drag_ext`
/// mod.
pub fn handle_window_moved_or_resized_end(
  window: &WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let Some(active_drag) = window.active_drag() else {
    return Ok(());
  };

  match &window {
    WindowContainer::NonTilingWindow(window) => {
      let is_maximized = try_warn!(window.native().is_maximized());

      window.update_native_properties(|properties| {
        properties.is_maximized = is_maximized;
      });

      let nearest_monitor = state
        .nearest_monitor(&window.native())
        .context("Failed to get workspace of nearest monitor.")?;

      let should_fullscreen = window.should_fullscreen(
        &nearest_monitor
          .displayed_workspace()
          .context("No workspace.")?,
      )?;

      if is_maximized || should_fullscreen {
        let fullscreen_state = if let WindowState::Fullscreen(
          fullscreen_state,
        ) = window.state()
        {
          fullscreen_state
        } else {
          config
            .value
            .window_behavior
            .state_defaults
            .fullscreen
            .clone()
        };

        let window = update_window_state(
          window.clone().into(),
          WindowState::Fullscreen(FullscreenStateConfig {
            maximized: is_maximized,
            ..fullscreen_state
          }),
          state,
          config,
        )?;

        window.set_active_drag(None);

        if is_maximized {
          // Dequeue the window from redraw if it's maximized, since the
          // window is already in the correct state.
          state
            .pending_sync
            .dequeue_container_from_redraw(window.clone());
        } else {
          // Force a redraw to snap the window to the monitor edges.
          // TODO: Skip redraw if it's already matches fullscreen frame.
          state.pending_sync.queue_container_to_redraw(window.clone());
        }

        return Ok(());
      }

      if active_drag.is_from_floating {
        update_floating_window_position(
          window,
          window.native_properties().frame,
          &nearest_monitor,
          state,
        )?;
        window.set_active_drag(None);
      } else {
        // Window is a temporary floating window that should be
        // reverted back to tiling.
        let window = drop_as_tiling_window(window, state, config)?;
        window.set_active_drag(None);
      }
    }
    WindowContainer::TilingWindow(window) => {
      tracing::info!(
        "Tiling window move/resize ended: {}",
        window.as_window_container()?
      );

      let frame = window.native_properties().frame;

      // Update the window's size based on the new frame position. This
      // means we use the actual window dimensions as the source of truth.
      set_window_size(
        window.clone().into(),
        Some(LengthValue::from_px(frame.width())),
        Some(LengthValue::from_px(frame.height())),
        state,
      )?;

      window.set_active_drag(None);

      // Force a redraw of the window to snap it back to its original
      // position. This is necessary when:
      // - The window is the only tiling window in the workspace.
      // - The window is not past the movement threshold for transitioning
      //   to floating while being dragged.
      // - Resizing in a direction that doesn't change the window's tiling
      //   size.
      state.pending_sync.queue_container_to_redraw(window.clone());
    }
  }

  Ok(())
}

/// Handles transition from temporary floating window to tiling window on
/// drag end.
#[allow(clippy::too_many_lines)]
fn drop_as_tiling_window(
  moved_window: &NonTilingWindow,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<WindowContainer> {
  tracing::info!(
    "Tiling window drag ended: {}",
    moved_window.as_window_container()?
  );

  let mouse_pos = state.dispatcher.cursor_position()?;
  let mouse_workspace = state
    .monitor_at_point(&mouse_pos)
    .and_then(|monitor| monitor.displayed_workspace())
    .or_else(|| moved_window.workspace())
    .context("Couldn't find workspace for window drop.")?;

  drop_into_tiling_tree(
    moved_window,
    &mouse_workspace.into(),
    &mouse_pos,
    state,
    config,
  )
}

#[allow(clippy::too_many_lines)]
fn drop_into_tiling_tree(
  moved_window: &NonTilingWindow,
  mouse_workspace: &DirectionContainer,
  mouse_pos: &Point,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<WindowContainer> {
  // Get the workspace, split containers, and other windows under the
  // dragged window.
  let containers_at_pos = state
    .containers_at_point(&mouse_workspace.clone().into(), mouse_pos)
    .into_iter()
    .filter(|container| container.id() != moved_window.id());

  // Get the deepest direction container under the dragged window.
  let target_parent: DirectionContainer = containers_at_pos
    .filter_map(|container| container.as_direction_container().ok())
    .fold(mouse_workspace.clone(), |acc, container| {
      if container.ancestors().count() > acc.ancestors().count() {
        container
      } else {
        acc
      }
    });

  // If the target parent has no children (i.e. an empty workspace), then
  // add the window directly.
  if target_parent.tiling_children().count() == 0 {
    move_container_within_tree(
      &moved_window.clone().into(),
      &target_parent.clone().into(),
      0,
      state,
    )?;

    moved_window.set_insertion_target(None);

    return update_window_state(
      moved_window.as_window_container()?,
      WindowState::Tiling,
      state,
      config,
    );
  }

  let nearest_container = target_parent
    .children()
    .into_iter()
    .filter_map(|container| container.as_tiling_container().ok())
    .try_fold(None, |acc: Option<TilingContainer>, container| match acc {
      Some(acc) => {
        let is_nearer = acc.to_rect()?.distance_to_point(mouse_pos)
          < container.to_rect()?.distance_to_point(mouse_pos);

        anyhow::Ok(Some(if is_nearer { acc } else { container }))
      }
      None => Ok(Some(container)),
    })?
    .context("No nearest container.")?;

  let tiling_direction = target_parent.tiling_direction();
  let drop_zone = drop_zone(mouse_pos, &nearest_container.to_rect()?);

  let moved_window = update_window_state(
    moved_window.clone().into(),
    WindowState::Tiling,
    state,
    config,
  )?;

  if drop_zone == DropZone::Center {
    stack_window(
      &moved_window,
      &nearest_container,
      &target_parent,
      state,
      config,
    )?;
  } else {
    let target_direction = drop_zone.tiling_direction();
    let is_before = drop_zone.is_before();

    if tiling_direction == target_direction {
      let target_index = if is_before {
        nearest_container.index()
      } else {
        nearest_container.index() + 1
      };

      move_container_within_tree(
        &moved_window.clone().into(),
        &target_parent.clone().into(),
        target_index,
        state,
      )?;
    } else {
      let split_container =
        SplitContainer::new(target_direction, config.value.gaps.clone());

      wrap_in_split_container(
        &split_container,
        &target_parent.clone().into(),
        std::slice::from_ref(&nearest_container),
      )?;

      let target_index = usize::from(!is_before);

      move_container_within_tree(
        &moved_window.clone().into(),
        &split_container.into(),
        target_index,
        state,
      )?;
    }
  }

  state.pending_sync.queue_container_to_redraw(target_parent);

  Ok(moved_window)
}

fn stack_window(
  moved_window: &WindowContainer,
  nearest_container: &TilingContainer,
  target_parent: &DirectionContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let stack_container = match nearest_container {
    TilingContainer::Stack(stack) => stack.clone(),
    TilingContainer::TilingWindow(_) | TilingContainer::Split(_) => {
      let stack_container = StackContainer::new(config.value.gaps.clone());
      wrap_in_stack_container(
        &stack_container,
        &target_parent.clone().into(),
        nearest_container,
      )?;
      stack_container
    }
  };

  move_container_within_tree(
    &moved_window.clone().into(),
    &stack_container.clone().into(),
    stack_container.child_count(),
    state,
  )?;

  set_focused_descendant(&moved_window.clone().into(), None);
  state
    .pending_sync
    .queue_container_to_redraw(stack_container)
    .queue_focus_change();

  Ok(())
}

/// Represents where the window was dropped over another.
#[derive(Debug, Clone, PartialEq)]
enum DropZone {
  Top,
  Bottom,
  Left,
  Right,
  Center,
}

impl DropZone {
  fn tiling_direction(&self) -> TilingDirection {
    match self {
      Self::Left | Self::Right => TilingDirection::Horizontal,
      Self::Top | Self::Bottom => TilingDirection::Vertical,
      Self::Center => {
        unreachable!("Center drops do not have a split direction.")
      }
    }
  }

  fn is_before(&self) -> bool {
    matches!(self, Self::Left | Self::Top)
  }
}

const CENTER_DROP_ZONE_RATIO: f64 = 0.4;

/// Gets the drop zone for a window based on the mouse position.
#[allow(clippy::cast_possible_truncation)]
fn drop_zone(mouse_pos: &Point, rect: &Rect) -> DropZone {
  let center_width = f64::from(rect.width()) * CENTER_DROP_ZONE_RATIO;
  let center_height = f64::from(rect.height()) * CENTER_DROP_ZONE_RATIO;
  let center_left =
    f64::from(rect.x()) + (f64::from(rect.width()) - center_width) / 2.0;
  let center_top =
    f64::from(rect.y()) + (f64::from(rect.height()) - center_height) / 2.0;
  let center_rect = Rect::from_xy(
    center_left.round() as i32,
    center_top.round() as i32,
    center_width.round() as i32,
    center_height.round() as i32,
  );

  if center_rect.contains_point(mouse_pos) {
    return DropZone::Center;
  }

  let delta_x = mouse_pos.x - rect.center_point().x;
  let delta_y = mouse_pos.y - rect.center_point().y;

  if delta_x.abs() > delta_y.abs() {
    // Window is in the left or right triangle.
    if delta_x > 0 {
      DropZone::Right
    } else {
      DropZone::Left
    }
  } else {
    // Window is in the top or bottom triangle.
    if delta_y > 0 {
      DropZone::Bottom
    } else {
      DropZone::Top
    }
  }
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_common::{ParsedConfig, TilingDirection, WorkspaceConfig};
  use wm_platform::{Dispatcher, Point, Rect};

  use super::{drop_into_tiling_tree, drop_zone, DropZone};
  use crate::{
    commands::{
      container::{
        attach_container, focus_tab, resize_tiling_container,
        set_focused_descendant, unstack, TabDirection,
      },
      monitor::ensure_workspaces_for_monitor,
      workspace::focus_all_workspaces,
    },
    models::{
      DirectionContainer, Monitor, NonTilingWindow, SplitContainer,
      StackContainer, TilingWindow, WindowContainer, Workspace,
    },
    traits::{CommonGetters, PositionGetters, TilingDirectionGetters},
    user_config::UserConfig,
    wm_state::WmState,
  };

  fn test_config() -> UserConfig {
    UserConfig::mock(ParsedConfig {
      workspaces: ["1", "2"]
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
  ) -> anyhow::Result<Monitor> {
    let monitor = Monitor::mock().call();
    attach_container(
      &monitor.clone().into(),
      &state.root_container.clone().into(),
      None,
    )?;
    ensure_workspaces_for_monitor(&monitor, state, config)?;
    Ok(monitor)
  }

  fn workspace(
    state: &WmState,
    monitor: &Monitor,
    name: &str,
  ) -> Workspace {
    state
      .workspace_by_name_in_monitor(monitor, name)
      .expect("workspace should exist")
  }

  fn tiling_window(title: &str) -> TilingWindow {
    TilingWindow::mock().title(title.to_string()).call()
  }

  fn floating_window(title: &str) -> NonTilingWindow {
    NonTilingWindow::mock().title(title.to_string()).call()
  }

  #[allow(clippy::needless_pass_by_value)]
  fn drop_window(
    moved: &NonTilingWindow,
    mouse_workspace: DirectionContainer,
    mouse_pos: Point,
    state: &mut WmState,
    config: &UserConfig,
  ) -> anyhow::Result<WindowContainer> {
    drop_into_tiling_tree(
      moved,
      &mouse_workspace,
      &mouse_pos,
      state,
      config,
    )
  }

  #[test]
  fn center_drop_creates_stack() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let target = tiling_window("A");
    let moved = floating_window("B");

    attach_container(
      &target.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    attach_container(
      &moved.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    set_focused_descendant(&target.clone().into(), None);

    let moved = drop_window(
      &moved,
      workspace.clone().into(),
      target.to_rect()?.center_point(),
      &mut state,
      &config,
    )?;

    let stack = moved
      .parent()
      .and_then(|parent| parent.as_stack().cloned())
      .expect("stack should be created");

    assert_eq!(stack.child_count(), 2);
    assert_eq!(stack.selected_child().expect("selected").id(), moved.id());

    Ok(())
  }

  #[test]
  fn center_drop_adds_to_existing_stack() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let moved = floating_window("C");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();

    attach_container(
      &stack.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    attach_container(
      &moved.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    set_focused_descendant(&a.clone().into(), None);

    let moved = drop_window(
      &moved,
      workspace.clone().into(),
      stack.to_rect()?.center_point(),
      &mut state,
      &config,
    )?;

    assert_eq!(stack.child_count(), 3);
    assert_eq!(stack.selected_child().expect("selected").id(), moved.id());

    Ok(())
  }

  #[test]
  fn edge_drop_creates_correct_split() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let target = tiling_window("A");
    let moved = floating_window("B");

    attach_container(
      &target.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    attach_container(
      &moved.clone().into(),
      &workspace.clone().into(),
      None,
    )?;

    let target_rect = target.to_rect()?;
    let top = Point {
      x: target_rect.center_point().x,
      y: target_rect.y() + 5,
    };

    drop_window(
      &moved,
      workspace.clone().into(),
      top,
      &mut state,
      &config,
    )?;

    assert_eq!(workspace.tiling_direction(), TilingDirection::Vertical);
    assert_eq!(workspace.child_count(), 2);

    Ok(())
  }

  #[test]
  fn edge_drop_reuses_existing_split() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let moved = floating_window("C");

    attach_container(&a.clone().into(), &workspace.clone().into(), None)?;
    attach_container(&b.clone().into(), &workspace.clone().into(), None)?;
    attach_container(
      &moved.clone().into(),
      &workspace.clone().into(),
      None,
    )?;

    let b_rect = b.to_rect()?;
    let right = Point {
      x: b_rect.x() + b_rect.width() - 5,
      y: b_rect.center_point().y,
    };

    drop_window(
      &moved,
      workspace.clone().into(),
      right,
      &mut state,
      &config,
    )?;

    assert_eq!(workspace.child_count(), 3);
    assert!(workspace.children().iter().all(|child| !child.is_split()));

    Ok(())
  }

  #[test]
  fn stack_occupies_single_parent_tile() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let c = tiling_window("C");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();

    attach_container(
      &stack.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    attach_container(&c.clone().into(), &workspace.clone().into(), None)?;

    let stack_rect = stack.to_rect()?;
    assert_eq!(a.to_rect()?, stack_rect);
    assert_eq!(b.to_rect()?, stack_rect);
    assert!(stack_rect.width() < workspace.to_rect()?.width());

    Ok(())
  }

  #[test]
  fn stack_next_prev_preserve_geometry() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();

    attach_container(
      &stack.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    set_focused_descendant(&a.clone().into(), None);
    let before = stack.to_rect()?;

    focus_tab(&a.clone().into(), TabDirection::Next, &mut state)?;
    assert_eq!(stack.selected_child().expect("selected").id(), b.id());
    assert_eq!(stack.to_rect()?, before);

    focus_tab(&b.clone().into(), TabDirection::Previous, &mut state)?;
    assert_eq!(stack.selected_child().expect("selected").id(), a.id());
    assert_eq!(stack.to_rect()?, before);

    Ok(())
  }

  #[test]
  fn stack_hidden_children_preserve_monitor_workspace(
  ) -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();

    attach_container(
      &stack.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    set_focused_descendant(&a.clone().into(), None);
    focus_tab(&a.clone().into(), TabDirection::Next, &mut state)?;

    assert_eq!(a.workspace().expect("workspace").id(), workspace.id());
    assert_eq!(a.monitor().expect("monitor").id(), monitor.id());
    assert_eq!(b.workspace().expect("workspace").id(), workspace.id());
    assert_eq!(b.monitor().expect("monitor").id(), monitor.id());

    Ok(())
  }

  #[test]
  fn unstack_collapses_single_child_stack() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();

    attach_container(
      &stack.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    set_focused_descendant(&a.clone().into(), None);

    unstack(&a.clone().into(), &mut state)?;

    assert!(stack.is_detached());
    assert_eq!(workspace.child_count(), 2);
    assert!(a
      .parent()
      .is_some_and(|parent| parent.id() == workspace.id()));
    assert!(b
      .parent()
      .is_some_and(|parent| parent.id() == workspace.id()));

    Ok(())
  }

  #[test]
  fn stack_survives_workspace_switch() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace_1 = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();

    attach_container(
      &stack.clone().into(),
      &workspace_1.clone().into(),
      None,
    )?;
    set_focused_descendant(&a.clone().into(), None);

    focus_all_workspaces("2", &mut state, &config)?;
    focus_all_workspaces("1", &mut state, &config)?;

    assert!(!stack.is_detached());
    assert_eq!(stack.child_count(), 2);
    assert_eq!(a.workspace().expect("workspace").id(), workspace_1.id());
    assert_eq!(b.workspace().expect("workspace").id(), workspace_1.id());

    Ok(())
  }

  #[test]
  fn nested_stack_resizes_with_parent_split() -> anyhow::Result<()> {
    let config = test_config();
    let mut state = test_state();
    let monitor = add_monitor(&mut state, &config)?;
    let workspace = workspace(&state, &monitor, "1");
    let a = tiling_window("A");
    let b = tiling_window("B");
    let c = tiling_window("C");
    let stack = StackContainer::mock()
      .tiling_containers(vec![a.clone().into(), b.clone().into()])
      .call();
    let split = SplitContainer::mock()
      .tiling_containers(vec![stack.clone().into(), c.clone().into()])
      .call();

    attach_container(
      &split.clone().into(),
      &workspace.clone().into(),
      None,
    )?;
    let before = stack.to_rect()?;
    resize_tiling_container(&stack.clone().into(), 0.7);
    let after = stack.to_rect()?;

    assert_ne!(before.width(), after.width());
    assert_eq!(a.to_rect()?, after);
    assert_eq!(b.to_rect()?, after);

    Ok(())
  }

  #[test]
  fn drop_zone_uses_center_and_nearest_edge() {
    let rect = Rect::from_xy(0, 0, 1000, 1000);

    assert_eq!(
      drop_zone(&Point { x: 500, y: 500 }, &rect),
      DropZone::Center
    );
    assert_eq!(drop_zone(&Point { x: 500, y: 20 }, &rect), DropZone::Top);
    assert_eq!(
      drop_zone(&Point { x: 980, y: 500 }, &rect),
      DropZone::Right
    );
  }
}
