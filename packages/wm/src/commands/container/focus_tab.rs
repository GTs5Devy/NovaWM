use anyhow::Context;

use super::{move_container_within_tree, set_focused_descendant};
use crate::{
  models::{Container, StackContainer},
  traits::CommonGetters,
  wm_state::WmState,
};

#[derive(Clone, Copy)]
pub enum TabDirection {
  Next,
  Previous,
}

pub fn focus_tab(
  origin_container: &Container,
  direction: TabDirection,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let Some(stack) = stack_ancestor(origin_container) else {
    return Ok(());
  };

  {
    let mut focus_order = stack.borrow_child_focus_order_mut();
    match direction {
      TabDirection::Next => {
        if let Some(front) = focus_order.pop_front() {
          focus_order.push_back(front);
        }
      }
      TabDirection::Previous => {
        if let Some(back) = focus_order.pop_back() {
          focus_order.push_front(back);
        }
      }
    }
  }

  let selected_child = stack
    .selected_child()
    .context("Stack has no selected child.")?;

  set_focused_descendant(&selected_child, None);
  state
    .pending_sync
    .queue_container_to_redraw(stack)
    .queue_focus_change();

  Ok(())
}

pub fn unstack(
  origin_container: &Container,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let Some(stack) = stack_ancestor(origin_container) else {
    return Ok(());
  };

  let window = origin_container
    .as_window_container()
    .ok()
    .or_else(|| {
      stack
        .selected_child()
        .and_then(|child| child.as_window_container().ok())
    })
    .context("No selected stack window.")?;

  let parent = stack.parent().context("Stack has no parent.")?;
  let target_index = stack.index() + 1;

  move_container_within_tree(
    &window.clone().into(),
    &parent,
    target_index,
    state,
  )?;

  set_focused_descendant(&window.into(), None);
  state
    .pending_sync
    .queue_container_to_redraw(parent)
    .queue_focus_change();

  Ok(())
}

fn stack_ancestor(container: &Container) -> Option<StackContainer> {
  container
    .self_and_ancestors()
    .find_map(|ancestor| ancestor.as_stack().cloned())
}
