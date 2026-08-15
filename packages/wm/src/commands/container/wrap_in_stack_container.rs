use anyhow::Context;
use wm_common::VecDequeExt;

use crate::{
  models::{Container, StackContainer, TilingContainer},
  traits::{CommonGetters, TilingSizeGetters},
};

pub fn wrap_in_stack_container(
  stack_container: &StackContainer,
  target_parent: &Container,
  target_child: &TilingContainer,
) -> anyhow::Result<()> {
  let target_index = target_child.index();
  let target_focus_index = target_child.focus_index();
  let target_tiling_size = target_child.tiling_size();

  target_parent
    .borrow_children_mut()
    .insert(target_index, stack_container.clone().into());
  target_parent
    .borrow_child_focus_order_mut()
    .insert(target_focus_index, stack_container.id());

  *stack_container.borrow_parent_mut() = Some(target_parent.clone());
  stack_container.set_tiling_size(target_tiling_size);

  *target_child.borrow_parent_mut() = Some(stack_container.clone().into());
  target_child.set_tiling_size(1.0);
  stack_container
    .borrow_children_mut()
    .push_back(target_child.clone().into());
  stack_container
    .borrow_child_focus_order_mut()
    .push_back(target_child.id());

  target_parent
    .borrow_children_mut()
    .retain(|child| child.id() != target_child.id());
  target_parent
    .borrow_child_focus_order_mut()
    .retain(|id| *id != target_child.id());

  target_parent
    .borrow_child_focus_order_mut()
    .shift_to_index(target_focus_index, stack_container.id());

  stack_container
    .parent()
    .context("Stack should be attached.")?;

  Ok(())
}
