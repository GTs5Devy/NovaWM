use std::collections::VecDeque;

use anyhow::Context;

use crate::{
  models::StackContainer,
  traits::{CommonGetters, TilingSizeGetters},
};

#[allow(clippy::needless_pass_by_value)]
pub fn collapse_stack_container(
  stack_container: StackContainer,
) -> anyhow::Result<()> {
  let Some(parent) = stack_container.parent() else {
    return Ok(());
  };

  match stack_container.child_count() {
    0 => {
      parent
        .borrow_children_mut()
        .retain(|child| child.id() != stack_container.id());
      parent
        .borrow_child_focus_order_mut()
        .retain(|id| *id != stack_container.id());
      *stack_container.borrow_parent_mut() = None;
    }
    1 => {
      let child = stack_container
        .children()
        .pop_front()
        .context("Stack should have one child.")?;
      let index = stack_container.index();
      let focus_index = stack_container.focus_index();

      *child.borrow_parent_mut() = Some(parent.clone());
      if let Ok(tiling_child) = child.as_tiling_container() {
        tiling_child.set_tiling_size(stack_container.tiling_size());
      }

      parent.borrow_children_mut().insert(index, child.clone());
      parent
        .borrow_child_focus_order_mut()
        .insert(focus_index, child.id());

      parent
        .borrow_children_mut()
        .retain(|child| child.id() != stack_container.id());
      parent
        .borrow_child_focus_order_mut()
        .retain(|id| *id != stack_container.id());

      *stack_container.borrow_parent_mut() = None;
      *stack_container.borrow_children_mut() = VecDeque::new();
      *stack_container.borrow_child_focus_order_mut() = VecDeque::new();
    }
    _ => {}
  }

  Ok(())
}
