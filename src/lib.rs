//! A behaviour tree system for bevy.
#![doc = include_str!("../readme.inc.md")]
#![deny(missing_docs)]
use std::{borrow::Cow, ops::RangeInclusive};

use bevy::prelude::*;
use ego_tree::*;

mod behave_trigger;
mod ctx;
mod dyn_bundle;
mod plugin;

#[cfg(test)]
mod tests;

use behave_trigger::*;
use ctx::*;
use dyn_bundle::prelude::*;

// in case users want to construct the tree without using the macro, we reexport:
pub use ego_tree;
use plugin::TickCtx;

/// Includes the ego_tree `tree!` macro for easy tree construction.
/// this crate also re-exports `ego_tree` so you can construct trees manually.
pub mod prelude {
    pub use super::behave;
    pub use super::behave_trigger::BehaveTrigger;
    pub use super::ctx::*;
    pub use super::plugin::*;
    pub use super::{Behave, BehaveFinished};
    pub use ego_tree::*;
}

/// A node on the behave tree can be in one of these states
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum BehaveNodeStatus {
    /// Node reported success
    Success,
    /// Node reported failure
    Failure,
    /// A task is in progress
    Running,
    /// Ticking to await a timer
    RunningTimer,
    /// Ticking suspended until trigger reports a status
    AwaitingTrigger,
    /// Next tick, reset node and descendants to initial state, for re-running.
    PendingReset,
}

/// Inserted on the entity with the BehaveTree when the tree has finished executing.
/// Containts the final result of the tree.
#[derive(Component, Reflect, Debug)]
pub struct BehaveFinished(pub bool);

/// A behaviour added to the tree by a user, which we convert to a a BehaviourNode tree internally
/// to run the tree. This is the template of the behaviour without all the internal runtime state.
///
/// Constuction is via static fns on Behave, so we can do the dynamic bundle stuff.
/// although this probably makes it hard to load the tree def from an asset file?
#[derive(Clone)]
pub enum Behave {
    /// Waits this many seconds before Succeeding
    Wait(f32),
    /// Spawns an entity, and waits for it to trigger a status report
    /// Use the Behaviour::dynamic_bundle fn to create.
    DynamicEntity {
        /// The name value of the Name component for this entity.
        name: Cow<'static, str>,
        /// The dynamic bundle to spawn.
        dynamic_bundel: DynamicBundel,
    },
    /// Runs children in sequence, failing if any fails, succeeding if all succeed
    Sequence,
    /// Runs children in sequence until one succeeds. If all fail, this fails.
    Fallback,
    /// Inverts success/failure of child. Must only have one child.
    Invert,
    /// Always succeeds
    AlwaysSucceed,
    /// Always fails
    AlwaysFail,
    /// Returns a result from a trigger. Can be used as a conditional (returning success or failure)
    /// or simply to execute some bevy systems code without spawning an entity.
    TriggerReq(DynamicTrigger),
    /// Loops forever
    Forever,
    /// Runs second child as long as first child succeeds, in a loop.
    While,
    /// If the first child succeeds, run the second child.
    /// (otherwise, run the third child, if present)
    IfThen,
    /// Runs all children in parallel, succeeding if all succeed, failing if any fail.
    WhenAll,
    /// Runs all children in parallel, succeeding if any child succeeds, failing if all fail.
    WhenAny,
}

impl std::fmt::Display for Behave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Behave::While => write!(f, "While"),
            Behave::Wait(secs) => write!(f, "Wait({secs}s)"),
            Behave::DynamicEntity { name, .. } => write!(f, "Spawn({name})"),
            Behave::Sequence => write!(f, "Sequence"),
            Behave::Fallback => write!(f, "Fallback"),
            Behave::Invert => write!(f, "Invert"),
            Behave::AlwaysSucceed => write!(f, "AlwaysSucceed"),
            Behave::AlwaysFail => write!(f, "AlwaysFail"),
            Behave::TriggerReq(t) => write!(f, "Trigger({})", t.type_name()),
            Behave::Forever => write!(f, "Forever"),
            Behave::IfThen => write!(f, "IfThen"),
            Behave::WhenAll => write!(f, "WhenAll"),
            Behave::WhenAny => write!(f, "WhenAny"),
        }
    }
}

impl Behave {
    /// Creates a new Behave::DynamicEntity, which means when this node runs, a new entity
    /// will be spawned with the components you provide in the `bundle` (as well as a [`BehaveCtx`]).
    pub fn spawn<T: Bundle + Clone>(bundle: T) -> Behave {
        Behave::DynamicEntity {
            name: "unnamed".into(),
            dynamic_bundel: DynamicBundel::new(bundle),
        }
    }
    /// Creates a new named Behave::DynamicEntity, which means when this node runs, a new entity
    /// will be spawned with:
    /// * the components you provide in the `bundle`
    /// * A bevy `Name` component with the value of `name`
    /// * [`BehaveCtx`]
    ///
    /// **NB** Don't include a `Name`` component in the bundle, since this fn adds on based on `name`.
    pub fn spawn_named<T: Bundle + Clone>(name: impl Into<Cow<'static, str>>, bundle: T) -> Behave {
        let name = name.into();
        Behave::DynamicEntity {
            name: name.clone(),
            dynamic_bundel: DynamicBundel::new((Name::new(name), bundle)),
        }
    }
    /// Creates a new Behave::TriggerReq, which means when this node runs, a trigger will be emitted
    /// using `BehaveTrigger<T>`. You can access the `value` in an observer using `trigger.event().inner()`.
    pub fn trigger<T: Clone + Send + Sync + 'static>(value: T) -> Self {
        Behave::TriggerReq(DynamicTrigger::new(value))
    }
    /// The permitted number of children for this node
    pub(crate) fn permitted_children(&self) -> RangeInclusive<usize> {
        match self {
            Behave::Sequence => 0..=usize::MAX,
            Behave::Fallback => 0..=usize::MAX,
            Behave::Forever => 1..=usize::MAX,
            Behave::While => 1..=2,
            Behave::IfThen => 2..=3,
            Behave::WhenAll => 0..=usize::MAX,
            Behave::WhenAny => 0..=usize::MAX,
            Behave::Invert => 1..=1,
            // Task nodes have no children:
            Behave::Wait(_) => 0..=0,
            Behave::TriggerReq(_) => 0..=0,
            Behave::DynamicEntity { .. } => 0..=0,
            // AlwaysSucceed and AlwaysFail are pseudo task nodes that don't have children:
            Behave::AlwaysSucceed => 0..=0,
            Behave::AlwaysFail => 0..=0,
        }
    }
}

/// A state wraps the behaviour, and is the node in our internal tree representation of the behaviour tree
/// One per Behave, with extra state bits.
#[derive(Clone)]
pub(crate) enum BehaveNode {
    Forever {
        status: Option<BehaveNodeStatus>,
    },
    Wait {
        start_time: Option<f32>,
        secs_to_wait: f32,
        status: Option<BehaveNodeStatus>,
    },
    DynamicEntity {
        // None until something spawned.
        task_status: EntityTaskStatus,
        status: Option<BehaveNodeStatus>,
        bundle: DynamicBundel,
        name: Cow<'static, str>,
    },
    SequenceFlow {
        status: Option<BehaveNodeStatus>,
    },
    FallbackFlow {
        status: Option<BehaveNodeStatus>,
    },
    Invert {
        status: Option<BehaveNodeStatus>,
    },
    AlwaysSucceed {
        status: Option<BehaveNodeStatus>,
    },
    AlwaysFail {
        status: Option<BehaveNodeStatus>,
    },
    TriggerReq {
        status: Option<BehaveNodeStatus>,
        task_status: TriggerTaskStatus,
        trigger: DynamicTrigger,
    },
    While {
        status: Option<BehaveNodeStatus>,
    },
    IfThen {
        status: Option<BehaveNodeStatus>,
    },
    WhenAll {
        status: Option<BehaveNodeStatus>,
    },
    WhenAny {
        status: Option<BehaveNodeStatus>,
    },
}

#[derive(Clone, Debug)]
enum EntityTaskStatus {
    NotStarted,
    Started(Entity),
    Complete(bool),
}

#[derive(Clone, Debug)]
enum TriggerTaskStatus {
    NotTriggered,
    Triggered,
    Complete(bool),
}

impl BehaveNode {
    fn status(&self) -> &Option<BehaveNodeStatus> {
        match self {
            BehaveNode::Forever { status } => status,
            BehaveNode::TriggerReq { status, .. } => status,
            BehaveNode::Wait { status, .. } => status,
            BehaveNode::DynamicEntity { status, .. } => status,
            BehaveNode::SequenceFlow { status } => status,
            BehaveNode::FallbackFlow { status } => status,
            BehaveNode::Invert { status } => status,
            BehaveNode::AlwaysSucceed { status } => status,
            BehaveNode::AlwaysFail { status } => status,
            BehaveNode::While { status } => status,
            BehaveNode::IfThen { status } => status,
            BehaveNode::WhenAll { status } => status,
            BehaveNode::WhenAny { status } => status,
        }
    }
    fn status_mut(&mut self) -> &mut Option<BehaveNodeStatus> {
        match self {
            BehaveNode::Forever { status } => status,
            BehaveNode::TriggerReq { status, .. } => status,
            BehaveNode::Wait { status, .. } => status,
            BehaveNode::DynamicEntity { status, .. } => status,
            BehaveNode::SequenceFlow { status } => status,
            BehaveNode::FallbackFlow { status } => status,
            BehaveNode::Invert { status } => status,
            BehaveNode::AlwaysSucceed { status } => status,
            BehaveNode::AlwaysFail { status } => status,
            BehaveNode::While { status } => status,
            BehaveNode::IfThen { status } => status,
            BehaveNode::WhenAll { status } => status,
            BehaveNode::WhenAny { status } => status,
        }
    }
}

impl std::fmt::Display for BehaveNode {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BehaveNode::Forever { .. } => write!(f, "Forever")?,
            BehaveNode::TriggerReq { trigger, .. } => write!(f, "TriggerReq({})", trigger.type_name())?,
            BehaveNode::Wait { secs_to_wait, .. } => write!(f, "Wait({secs_to_wait})")?,
            BehaveNode::DynamicEntity {name, .. } => write!(f, "DynamicEntity({name})")?,
            BehaveNode::SequenceFlow { .. } => write!(f, "SequenceFlow")?,
            BehaveNode::FallbackFlow { .. } => write!(f, "FallbackFlow")?,
            BehaveNode::Invert { .. } => write!(f, "Invert")?,
            BehaveNode::AlwaysSucceed { .. } => write!(f, "AlwaysSucceed")?,
            BehaveNode::AlwaysFail { .. } => write!(f, "AlwaysFail")?,
            BehaveNode::While { .. } => write!(f, "While")?,
            BehaveNode::IfThen { .. } => write!(f, "IfThen")?,
            BehaveNode::WhenAll { .. } => write!(f, "WhenAll")?,
            BehaveNode::WhenAny { .. } => write!(f, "WhenAny")?,
        }
        match self.status() {
            Some(BehaveNodeStatus::Success) => write!(f, " --> ✅"),
            Some(BehaveNodeStatus::Failure) => write!(f, " --> ❌"),
            Some(BehaveNodeStatus::Running) => write!(f, " --> ⏳"),
            Some(BehaveNodeStatus::RunningTimer) => write!(f, " --> ⏳"),
            Some(BehaveNodeStatus::AwaitingTrigger) => write!(f, " --> ⏳"),
            Some(BehaveNodeStatus::PendingReset) => write!(f, " --> 🔄"),
            _ => Ok(()),
        }
    }
}

impl BehaveNode {
    pub(crate) fn reset(&mut self) {
        match self {
            BehaveNode::Forever { status } => {
                *status = None;
            }
            BehaveNode::TriggerReq {
                status,
                task_status,
                ..
            } => {
                *status = None;
                *task_status = TriggerTaskStatus::NotTriggered;
            }
            BehaveNode::Wait {
                status, start_time, ..
            } => {
                *status = None;
                *start_time = None;
            }
            BehaveNode::DynamicEntity {
                status,
                task_status,
                ..
            } => {
                *status = None;
                *task_status = EntityTaskStatus::NotStarted;
            }
            BehaveNode::SequenceFlow { status } => {
                *status = None;
            }
            BehaveNode::FallbackFlow { status } => {
                *status = None;
            }
            BehaveNode::Invert { status } => {
                *status = None;
            }
            BehaveNode::AlwaysSucceed { status } => {
                *status = None;
            }
            BehaveNode::AlwaysFail { status } => {
                *status = None;
            }
            BehaveNode::While { status } => {
                *status = None;
            }
            BehaveNode::IfThen { status } => {
                *status = None;
            }
            BehaveNode::WhenAll { status } => {
                *status = None;
            }
            BehaveNode::WhenAny { status } => {
                *status = None;
            }
        }
    }
    pub(crate) fn new(behave: Behave) -> Self {
        match behave {
            Behave::Forever => Self::Forever { status: None },
            Behave::TriggerReq(trig_fn) => Self::TriggerReq {
                status: None,
                task_status: TriggerTaskStatus::NotTriggered,
                trigger: trig_fn,
            },
            Behave::Wait(secs_to_wait) => Self::Wait {
                start_time: None,
                secs_to_wait,
                status: None,
            },
            Behave::DynamicEntity {
                name,
                dynamic_bundel: bundle,
            } => Self::DynamicEntity {
                task_status: EntityTaskStatus::NotStarted,
                status: None,
                bundle,
                name,
            },
            Behave::While => Self::While { status: None },
            Behave::Sequence => Self::SequenceFlow { status: None },
            Behave::Fallback => Self::FallbackFlow { status: None },
            Behave::Invert => Self::Invert { status: None },
            Behave::AlwaysSucceed => Self::AlwaysSucceed { status: None },
            Behave::AlwaysFail => Self::AlwaysFail { status: None },
            Behave::IfThen => Self::IfThen { status: None },
            Behave::WhenAll => Self::WhenAll { status: None },
            Behave::WhenAny => Self::WhenAny { status: None },
        }
    }
}

// sucks there aren't good traversal fns on NodeMut like there are on NodeRef..
fn reset_descendants(n: &mut NodeMut<BehaveNode>) {
    // info!("Restting node: {:?}", n.id());
    n.value().reset();
    if let Some(mut sibling) = n.next_sibling() {
        reset_descendants(&mut sibling);
    }
    if let Some(mut child) = n.first_child() {
        reset_descendants(&mut child);
    }
}

fn tick_node(
    n: &mut NodeMut<BehaveNode>,
    commands: &mut Commands,
    tick_ctx: &TickCtx,
) -> BehaveNodeStatus {
    use BehaveNode::*;
    // if logging {
    //     info!("tick_node: {:?} = {}", n.id(), n.value());
    // }
    // short circuit nodes that have already got a result
    let reset_needed = match n.value().status() {
        Some(BehaveNodeStatus::Success) => return BehaveNodeStatus::Success,
        Some(BehaveNodeStatus::Failure) => return BehaveNodeStatus::Failure,
        Some(BehaveNodeStatus::PendingReset) => true,
        _ => false,
    };
    if reset_needed {
        *n.value().status_mut() = Some(BehaveNodeStatus::Running);
        reset_descendants(n);
    }
    let task_node = n.id();
    match n.value() {
        While { .. } => {
            *n.value().status_mut() = Some(BehaveNodeStatus::Running);
            let mut first_child = n
                .first_child()
                .expect("While node first child must exist (the conditional)");
            match tick_node(&mut first_child, commands, tick_ctx) {
                BehaveNodeStatus::Success => {
                    *first_child.value().status_mut() = Some(BehaveNodeStatus::Success);
                    // if the conditional succeeds, we run the second child if present.
                    // also supported a while node with just one child, which will simply repeat
                    // until that child fails.
                    if let Some(mut second_child) = first_child.next_sibling() {
                        match tick_node(&mut second_child, commands, tick_ctx) {
                            BehaveNodeStatus::Success => {
                                *second_child.value().status_mut() =
                                    Some(BehaveNodeStatus::Success);
                                // if the body succeeds, we loop back to the conditional
                                *n.value().status_mut() = Some(BehaveNodeStatus::PendingReset);
                                BehaveNodeStatus::PendingReset
                            }
                            // if body is resetting, don't reset the whole loop
                            BehaveNodeStatus::PendingReset => BehaveNodeStatus::Running,
                            other => {
                                // failing second node doesn't matter, we dont care. always run again.
                                *n.value().status_mut() = Some(other);
                                other
                            }
                        }
                    } else {
                        *n.value().status_mut() = Some(BehaveNodeStatus::PendingReset);
                        BehaveNodeStatus::PendingReset
                    }
                }
                BehaveNodeStatus::PendingReset => BehaveNodeStatus::Running,
                other => {
                    *first_child.value().status_mut() = Some(other);
                    *n.value().status_mut() = Some(other);
                    other
                }
            }
        }
        IfThen { .. } => {
            *n.value().status_mut() = Some(BehaveNodeStatus::Running);
            let mut conditional_child = n
                .first_child()
                .expect("IfThen node first child must exist (the 'if condition' child)");
            // evaluate the condition child
            match tick_node(&mut conditional_child, commands, tick_ctx) {
                BehaveNodeStatus::Success => {
                    // the condition child succeeded, so the If node returns the result of evaluating the then child.
                    *conditional_child.value().status_mut() = Some(BehaveNodeStatus::Success);
                    let mut then_child = conditional_child
                        .next_sibling()
                        .expect("IfThen node second child must exist (the 'then' child)");
                    let then_result = tick_node(&mut then_child, commands, tick_ctx);
                    *n.value().status_mut() = Some(then_result);
                    then_result
                }
                BehaveNodeStatus::Failure => {
                    // the condition child failed, an "else" child is optional. run if present:
                    *conditional_child.value().status_mut() = Some(BehaveNodeStatus::Failure);
                    if let Some(mut else_child) = conditional_child
                        .next_sibling()
                        .expect("If nodes must have exactly two or three children")
                        .next_sibling()
                    {
                        // if there is an else child, the If node returns the result of evaluating the else child.
                        let else_result = tick_node(&mut else_child, commands, tick_ctx);
                        *n.value().status_mut() = Some(else_result);
                        else_result
                    } else {
                        // if no else child, and conditional fails, the If node fails.
                        *n.value().status_mut() = Some(BehaveNodeStatus::Failure);
                        BehaveNodeStatus::Failure
                    }
                }
                BehaveNodeStatus::PendingReset => BehaveNodeStatus::Running,
                other => {
                    *n.value().status_mut() = Some(other);
                    other
                }
            }
        }
        Forever { .. } => {
            *n.value().status_mut() = Some(BehaveNodeStatus::Running);
            let mut only_child = n.first_child().expect("Forever nodes must have a child");
            if only_child.has_siblings() {
                panic!("Forever nodes must have a single child, not multiple children");
            }
            match tick_node(&mut only_child, commands, tick_ctx) {
                // if our child node completes, reset next tick so we can run it again
                BehaveNodeStatus::Success | BehaveNodeStatus::Failure => {
                    *n.value().status_mut() = Some(BehaveNodeStatus::PendingReset);
                    BehaveNodeStatus::PendingReset
                }
                BehaveNodeStatus::PendingReset => BehaveNodeStatus::Running,
                other => other,
            }
        }
        TriggerReq {
            task_status: task_status @ TriggerTaskStatus::NotTriggered,
            status,
            trigger,
        } => {
            let ctx = BehaveCtx::new_for_trigger(task_node, tick_ctx);
            commands.dyn_trigger(trigger.clone(), ctx);
            // Don't use AwaitingTrigger for this, because of ordering issues..
            // the trigger response arrives BEFORE we insert the BehaveAwaitingTrigger component,
            // so the trigger response handler can't remove it, so it never ticks.
            *task_status = TriggerTaskStatus::Triggered;
            *status = Some(BehaveNodeStatus::Running);
            BehaveNodeStatus::Running
        }
        #[rustfmt::skip]
        TriggerReq {task_status: TriggerTaskStatus::Complete(true), status, ..} => {
            *status = Some(BehaveNodeStatus::Success);
            BehaveNodeStatus::Success
        }
        #[rustfmt::skip]
        TriggerReq {task_status: TriggerTaskStatus::Complete(false), status, ..} => {
            *status = Some(BehaveNodeStatus::Failure);
            BehaveNodeStatus::Failure
        }
        // in this case, the trigger didn't report a result immediately, so we go into awaiting trigger mode.
        // this happens if the trigger hands the ctx off to another system to report the result later.
        #[rustfmt::skip]
        TriggerReq {task_status: TriggerTaskStatus::Triggered, status, .. } => {
            *status = Some(BehaveNodeStatus::AwaitingTrigger);
            BehaveNodeStatus::AwaitingTrigger
        }
        Invert { .. } => {
            let mut only_child = n.first_child().expect("Invert nodes must have a child");
            if only_child.has_siblings() {
                panic!("Invert nodes must have a single child, not multiple children");
            }
            let res = match tick_node(&mut only_child, commands, tick_ctx) {
                BehaveNodeStatus::Success => BehaveNodeStatus::Failure, // swapped
                BehaveNodeStatus::Failure => BehaveNodeStatus::Success, // swapped
                BehaveNodeStatus::PendingReset => BehaveNodeStatus::Running,
                other => other,
            };
            let Invert { status } = n.value() else {
                unreachable!("Must be an Invert");
            };
            *status = Some(res);
            res
        }
        AlwaysSucceed { status } => {
            *status = Some(BehaveNodeStatus::Success);
            BehaveNodeStatus::Success
        }
        AlwaysFail { status } => {
            *status = Some(BehaveNodeStatus::Failure);
            BehaveNodeStatus::Failure
        }
        // start waiting
        Wait {
            start_time: start_time @ None,
            status,
            ..
        } => {
            // info!("Starting wait");
            *start_time = Some(tick_ctx.elapsed_secs);
            *status = Some(BehaveNodeStatus::Running);
            BehaveNodeStatus::Running
        }
        // continue waiting
        Wait {
            start_time: Some(start_time),
            secs_to_wait,
            status,
        } => {
            // info!("Waiting");
            let elapsed = tick_ctx.elapsed_secs - *start_time;
            if elapsed > *secs_to_wait {
                *status = Some(BehaveNodeStatus::Success);
                return BehaveNodeStatus::Success;
            }
            BehaveNodeStatus::RunningTimer
        }
        // spawn a new entity for this task
        DynamicEntity {
            task_status: task_status @ EntityTaskStatus::NotStarted,
            status,
            bundle,
            name: _,
        } => {
            let mut e = commands.spawn(());
            e.insert(ChildOf(tick_ctx.bt_entity));
            let ctx = BehaveCtx::new_for_entity(task_node, tick_ctx, e.id());
            // NB: if the component in the dyn bundle has an OnAdd which reports success or failure
            //     immediately, the entity will be despawned instantly, so you can't do something
            //     like .set_parent on it after doing the insertion (we set_parent above).
            //     Else you get a "The entity with ID X does not exist" panic in bevy_hierarchy code.
            let id = e.dyn_insert(bundle.clone(), Some(ctx)).id();
            // info!("Spawned entity: {id:?} (parent: {bt_entity:?}) for node {task_node:?}",);
            *task_status = EntityTaskStatus::Started(id);
            // We go to Running for one tick, so that any OnAdd trigger that immediately reports a
            // status we take effect properly.
            // Next match case will set to AwaitingTrigger if we don't get a status report
            // Otherwise there is an ordering mismatch and the AwaitingTrigger isn't removed.
            *status = Some(BehaveNodeStatus::Running);
            BehaveNodeStatus::Running
        }
        #[rustfmt::skip]
        DynamicEntity { task_status: EntityTaskStatus::Started(_), status: status @ Some(BehaveNodeStatus::Running), .. } => {
            // if we tick without having received a status report, it means there can't have been any OnAdd trigger 
            // that immediately sent a report and caused a despawn, so we can go dormant:
            *status = Some(BehaveNodeStatus::AwaitingTrigger);
            BehaveNodeStatus::AwaitingTrigger
        }
        #[rustfmt::skip]
        DynamicEntity{ task_status: EntityTaskStatus::Started(_), .. } => {
            // Can be ticked multiple times while awaiting a trigger (e.g. under WhenAll).
            // Stay in AwaitingTrigger until a status report arrives.
            BehaveNodeStatus::AwaitingTrigger
        }
        // this is when a trigger has reported a result, and we need to process it and update status
        #[rustfmt::skip]
        DynamicEntity {task_status: EntityTaskStatus::Complete(true), status, ..} => {
            *status = Some(BehaveNodeStatus::Success);
            BehaveNodeStatus::Success
        }
        #[rustfmt::skip]
        DynamicEntity {task_status: EntityTaskStatus::Complete(false), status, ..} => {
            *status = Some(BehaveNodeStatus::Failure);
            BehaveNodeStatus::Failure
        }
        // don't bind any fields here because we need to mutably borrow the node again
        SequenceFlow { .. } => {
            // info!("SequenceFlow. Processing children");
            let Some(mut child) = n.first_child() else {
                warn!("SequenceFlow with no children, returning success anyway");
                return BehaveNodeStatus::Success;
            };

            let mut final_status;
            loop {
                match tick_node(&mut child, commands, tick_ctx) {
                    BehaveNodeStatus::Success => {
                        final_status = BehaveNodeStatus::Success;
                        if let Ok(next_child) = child.into_next_sibling() {
                            child = next_child;
                            continue;
                        } else {
                            break;
                        }
                    }
                    BehaveNodeStatus::PendingReset => {
                        final_status = BehaveNodeStatus::Running;
                        break;
                    }
                    // A non-success state just gets bubbled up to the parent
                    other => {
                        final_status = other;
                        break;
                    }
                }
            }
            let SequenceFlow { status, .. } = n.value() else {
                unreachable!("Must be a SequenceFlow");
            };
            *status = Some(final_status);
            final_status
        }

        FallbackFlow { .. } => {
            let Some(mut child) = n.first_child() else {
                warn!("FallbackFlow with no children, returning success anyway");
                return BehaveNodeStatus::Success;
            };

            let mut final_status;
            loop {
                match tick_node(&mut child, commands, tick_ctx) {
                    BehaveNodeStatus::Failure => {
                        // a child fails, try the next one, or if no more children, we failed.
                        final_status = BehaveNodeStatus::Failure;
                        if let Ok(next_child) = child.into_next_sibling() {
                            child = next_child;
                            continue;
                        } else {
                            break;
                        }
                    }
                    BehaveNodeStatus::PendingReset => {
                        final_status = BehaveNodeStatus::Running;
                        break;
                    }
                    // A non-failure state just gets bubbled up to the parent
                    other => {
                        final_status = other;
                        break;
                    }
                }
            }
            let FallbackFlow { status, .. } = n.value() else {
                unreachable!("Must be a FallbackFlow");
            };
            *status = Some(final_status);
            final_status
        }
        WhenAll { .. } => {
            *n.value().status_mut() = Some(BehaveNodeStatus::Running);
            let mut any_pending = false;
            let mut any_failed = false;

            let Some(mut child) = n.first_child() else {
                *n.value().status_mut() = Some(BehaveNodeStatus::Success);
                return BehaveNodeStatus::Success;
            };

            loop {
                let child_status = child.value().status().clone();
                let child_done = match child_status {
                    Some(BehaveNodeStatus::Success) => true,
                    Some(BehaveNodeStatus::Failure) => {
                        any_failed = true;
                        true
                    }
                    _ => false,
                };

                if !child_done {
                    let result = tick_node(&mut child, commands, tick_ctx);
                    match result {
                        BehaveNodeStatus::Success => {}
                        BehaveNodeStatus::Failure => {
                            any_failed = true;
                        }
                        _ => {
                            any_pending = true;
                        }
                    }
                }

                if let Ok(next_child) = child.into_next_sibling() {
                    child = next_child;
                } else {
                    break;
                }
            }

            if any_pending {
                *n.value().status_mut() = Some(BehaveNodeStatus::Running);
                BehaveNodeStatus::Running
            } else {
                let final_status = if any_failed {
                    BehaveNodeStatus::Failure
                } else {
                    BehaveNodeStatus::Success
                };
                *n.value().status_mut() = Some(final_status);
                final_status
            }
        }
        WhenAny { .. } => {
            let mut all_failed = true;

            let Some(mut child) = n.first_child() else {
                *n.value().status_mut() = Some(BehaveNodeStatus::Success);
                return BehaveNodeStatus::Success;
            };

            loop {
                let child_status = child.value().status().clone();
                match child_status {
                    Some(BehaveNodeStatus::Success) => {
                        *n.value().status_mut() = Some(BehaveNodeStatus::Success);
                        return BehaveNodeStatus::Success;
                    }
                    Some(BehaveNodeStatus::Failure) => {}
                    _ => {
                        all_failed = false;
                        let result = tick_node(&mut child, commands, tick_ctx);
                        match result {
                            BehaveNodeStatus::Success => {
                                *n.value().status_mut() = Some(BehaveNodeStatus::Success);
                                return BehaveNodeStatus::Success;
                            }
                            BehaveNodeStatus::Failure => {}
                            _ => {}
                        }
                    }
                }

                if let Ok(next_child) = child.into_next_sibling() {
                    child = next_child;
                } else {
                    break;
                }
            }

            if all_failed {
                *n.value().status_mut() = Some(BehaveNodeStatus::Failure);
                BehaveNodeStatus::Failure
            } else {
                *n.value().status_mut() = Some(BehaveNodeStatus::Running);
                BehaveNodeStatus::Running
            }
        }
    }
}

/// Modifed version of ego_tree's tree! macro, to allow merging subtrees:
///
/// let subtree: Tree<Behave> = get_subtree();
/// let t = tree! {
///     Behave::Sequence => {
///         Behave::Wait(2),
///         @ subtree
///     }
/// };
///
/// Also supports appending a list of children from an iterator:
///
/// let children: Vec<Behave> = get_children();
/// let t = tree! {
///     Behave::Sequence => {
///         @[ children ]
///     }
/// };
#[macro_export]
macro_rules! behave {
    // Append a bunch of children from an iterator
    (@ $n:ident { @[ $children:expr ] $(, $($tail:tt)*)? }) => {{
        for child in $children {
            $n.append(child);
        }
        $( behave!(@ $n { $($tail)* }); )?
    }};

    // Append a bunch of trees from an iterator
    (@ $n:ident { ... $children:expr $(, $($tail:tt)*)? }) => {{
        for child in $children {
            $n.append_subtree(child);
        }
        $( behave!(@ $n { $($tail)* }); )?
    }};

    // Use an “@” marker to indicate that the expression is a subtree, to be merged into the tree.
    (@ $n:ident { @ $subtree:expr $(, $($tail:tt)*)? }) => {{
        $n.append_subtree($subtree);
        $( behave!(@ $n { $($tail)* }); )?
    }};

    // Base case: no tokens left, with optional trailing comma.
    (@ $n:ident { $(,)? }) => { };

    // Leaf: last value, with optional trailing comma.
    (@ $n:ident { $value:expr $(,)? }) => {{
        $n.append($value);
    }};

    // Leaf: value with additional siblings.
    (@ $n:ident { $value:expr, $($tail:tt)* }) => {{
        $n.append($value);
        behave!(@ $n { $($tail)* });
    }};

    // Node: last node with children, with optional trailing comma.
    (@ $n:ident { $value:expr => $children:tt $(,)? }) => {{
        let mut node = $n.append($value);
        behave!(@ node $children);
    }};

    // Node: node with children and additional siblings.
    (@ $n:ident { $value:expr => $children:tt, $($tail:tt)* }) => {{
        let mut node = $n.append($value);
        behave!(@ node $children);
        behave!(@ $n { $($tail)* });
    }};

    // Top-level: tree with a root only.
    ($root:expr $(,)?) => { $crate::ego_tree::Tree::new($root) };

    // Top-level: tree with a root and children, with optional trailing comma.
    ($root:expr => $children:tt $(,)?) => {{
        let mut tree = $crate::ego_tree::Tree::new($root);
        {
            // unused in empty sequence or fallback nodes
            #[allow(unused)]
            let mut node = tree.root_mut();
            behave!(@ node $children);
        }
        tree
    }};
}
