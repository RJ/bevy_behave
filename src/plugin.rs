use crate::{
    BehaveNode, BehaveNodeStatus, EntityTaskStatus, TriggerTaskStatus, prelude::*, tick_node,
};
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
// use bevy::app::FixedPreUpdate;
use bevy::ecs::intern::Interned;
use bevy::ecs::schedule::{ScheduleLabel, SystemSet};
use ego_tree::*;

use crate::ctx::BehaveDespawnTaskEntity;

/// The `BehaveTree` components are ticked in this set, which is configured into the schedule
/// provided to the `BehavePlugin`. This defaults to `FixedPreUpdate`.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct BehaveSet;

/// Plugin to tick the `BehaveTree` components.
/// Defaults to configuring the `BehaveSet` to run in `FixedPreUpdate`.
pub struct BehavePlugin {
    schedule: Interned<dyn ScheduleLabel>,
    /// if true, use an exclusive mut World system to tick trees, to avoid next-frame delays on triggers
    synchronous: bool,
}

impl BehavePlugin {
    /// Run the BehaveTree tick system in this schedule
    pub fn new(schedule: impl ScheduleLabel) -> Self {
        Self {
            schedule: schedule.intern(),
            synchronous: false,
        }
    }
    /// Return the schedule this plugin will run in.
    pub fn schedule(&self) -> &Interned<dyn ScheduleLabel> {
        &self.schedule
    }

    /// Enables use of exclusive system to tick the trees
    /// (to avoid next-frame delays on triggers)
    pub fn with_synchronous(mut self) -> Self {
        self.synchronous = true;
        self
    }
}

impl Default for BehavePlugin {
    /// Defaults to `FixedPreUpdate`.
    fn default() -> Self {
        Self::new(FixedPreUpdate)
    }
}

impl Plugin for BehavePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(self.schedule, BehaveSet);
        app.register_type::<BehaveTimeout>();

        // Clean up any task entities that completed on a prior tick.
        // (We deliberately defer despawn to avoid command-application ordering issues.)
        app.add_systems(
            self.schedule,
            despawn_completed_task_entities.in_set(BehaveSet),
        );

        app.add_systems(self.schedule, tick_timeout_components.in_set(BehaveSet));

        if self.synchronous {
            warn!("Using experimental synchronous tree ticking");
            app.add_systems(
                self.schedule,
                tick_trees_sync
                    .after(tick_timeout_components)
                    .in_set(BehaveSet),
            );
        } else {
            app.add_systems(
                self.schedule,
                tick_trees.after(tick_timeout_components).in_set(BehaveSet),
            );
        }

        app.add_observer(on_tick_timeout_added);
        // adds a global observer to listen for status report events
        app.add_plugins(crate::ctx::plugin);
    }
}

fn despawn_completed_task_entities(
    q: Query<Entity, Added<BehaveDespawnTaskEntity>>,
    mut commands: Commands,
) {
    for e in q.iter() {
        commands.entity(e).try_despawn();
    }
}

/// The entity of the character that the behaviour is controlling.
/// This is required to be on the entity holding the BehaviourTree component.
/// The actual entity (either specified here or the parent) is provided by calling
/// `ctx.target_entity()` from the ctx component or trigger event.
#[derive(Component, Debug, Default, Clone)]
pub enum BehaveTargetEntity {
    /// Uses the direct parent of the behaviour tree entity as the target entity.
    #[default]
    Parent,
    /// Finds the root ancestor of the behaviour tree entity and uses that as the target entity.
    RootAncestor,
    /// Always returns the specified entity as the target entity.
    Entity(Entity),
}

/// Tracks the entity of the supervisor that is controlling the behaviour tree.
/// Only used when running under my unreleased HTN crate that complements bevy_behave.
#[derive(Component, Debug)]
pub struct BehaveSupervisorEntity(pub Entity);

/// If present on the BehaveTree entity, don't tick tree.
/// Means tree is sleeping, until a trigger reports a status (which removes the component).
#[derive(Component)]
pub(crate) struct BehaveAwaitingTrigger;

#[allow(clippy::type_complexity)]
fn tick_trees(
    mut query: Query<
        (
            Entity,
            &mut BehaveTree,
            Option<&ChildOf>,
            &BehaveTargetEntity,
            Option<&BehaveSupervisorEntity>,
        ),
        (Without<BehaveAwaitingTrigger>, Without<BehaveFinished>),
    >,
    q_parents: Query<&ChildOf>,
    mut commands: Commands,
    time: Res<Time>,
) {
    for (bt_entity, mut bt, opt_parent, target_entity, opt_sup_entity) in query.iter_mut() {
        let target_entity = match target_entity {
            BehaveTargetEntity::Parent => opt_parent
                .map(|p| p.parent())
                .unwrap_or(Entity::PLACEHOLDER),
            BehaveTargetEntity::Entity(e) => *e,
            BehaveTargetEntity::RootAncestor => q_parents.root_ancestor(bt_entity),
        };
        let tick_ctx = TickCtx::new(bt_entity, target_entity, time.elapsed_secs())
            .with_optional_sup_entity(opt_sup_entity.map(|c| c.0));
        let tick_result = bt.tick(&mut commands, &tick_ctx);
        match tick_result {
            BehaveNodeStatus::AwaitingTrigger => {
                commands.entity(bt_entity).insert(BehaveAwaitingTrigger);
            }
            BehaveNodeStatus::Success => {
                commands.entity(bt_entity).insert(BehaveFinished(true));
            }
            BehaveNodeStatus::Failure => {
                commands.entity(bt_entity).insert(BehaveFinished(false));
            }
            BehaveNodeStatus::RunningTimer => {}
            BehaveNodeStatus::Running => {}
            BehaveNodeStatus::PendingReset => {}
        }
        if bt.logging && tick_result != BehaveNodeStatus::RunningTimer {
            info!("ticked tree(async): {bt_entity}\n{}", bt.tree);
        }
    }
}

const SANITY_LOOP_LIMIT: usize = 1000;

/// An exclusive mut World system version of tick_trees.
///
/// Since the query filter means we're only finding trees that are A) not finished and B) not waiting on a trigger response,
/// we can just keep on ticking any trees the query finds until it's empty.
///
/// This means that if you have a tree with a Behave::trigger(Whatever), which returns immediately,
/// (eg, the observer reports the status via commands.trigger), it will be re-ticked immediately,
/// and progress to the next node, without any next-frame delay.
#[allow(clippy::type_complexity)]
fn tick_trees_sync(
    world: &mut World,
    params: &mut SystemState<(
        Query<
            (
                Entity,
                &mut BehaveTree,
                Option<&ChildOf>,
                &BehaveTargetEntity,
                Option<&BehaveSupervisorEntity>,
            ),
            (Without<BehaveAwaitingTrigger>, Without<BehaveFinished>),
        >,
        Query<&ChildOf>,
        Commands,
        Res<Time>,
    )>,
) {
    let mut sanity_counter = 0;
    loop {
        let (mut query, q_parents, mut commands, time) = params.get_mut(world);
        if query.is_empty() {
            return;
        }
        sanity_counter += 1;
        // avoid infinite loops in case of logic errors:
        if sanity_counter > SANITY_LOOP_LIMIT {
            error!("SANITY_LOOP_LIMIT counter exceeded! aborting tick loop");
            break;
        }
        // info!("Ticking {} trees (sync)", query.iter().count());

        let mut trees_processed = 0;
        for (bt_entity, mut bt, opt_parent, target_entity, opt_sup_entity) in query.iter_mut() {
            let target_entity = match target_entity {
                BehaveTargetEntity::Parent => opt_parent
                    .map(|p| p.parent())
                    .unwrap_or(Entity::PLACEHOLDER),
                BehaveTargetEntity::Entity(e) => *e,
                BehaveTargetEntity::RootAncestor => q_parents.root_ancestor(bt_entity),
            };
            let tick_ctx = TickCtx::new(bt_entity, target_entity, time.elapsed_secs())
                .with_optional_sup_entity(opt_sup_entity.map(|c| c.0));
            let tick_result = bt.tick(&mut commands, &tick_ctx);
            match tick_result {
                BehaveNodeStatus::AwaitingTrigger => {
                    commands.entity(bt_entity).insert(BehaveAwaitingTrigger);
                }
                BehaveNodeStatus::Success => {
                    commands.entity(bt_entity).insert(BehaveFinished(true));
                }
                BehaveNodeStatus::Failure => {
                    commands.entity(bt_entity).insert(BehaveFinished(false));
                }
                BehaveNodeStatus::RunningTimer => {}
                BehaveNodeStatus::Running => {}
                BehaveNodeStatus::PendingReset => {}
            }
            if bt.logging && tick_result != BehaveNodeStatus::RunningTimer {
                info!("ticked tree (sync): {bt_entity}\n{}", bt.tree);
            }
            // trees that are waiting on a timer will always be happy to tick, but they don't need to
            // be ticked more than once per frame, since the time won't advance until the next frame.
            // so RunningTimer results don't increment the trees_processed counter.
            if tick_result != BehaveNodeStatus::RunningTimer {
                trees_processed += 1;
            }
        }
        params.apply(world);
        if trees_processed == 0 {
            // either no trees, or all trees are running timers and don't need to be re-ticked
            // until next frame.
            break;
        }
    }
}

/// The main behaviour tree component.
/// A `bevy_behave` system will query all entities with a `BehaveTree` to tick them.
/// (unless they have a `BehaveAwaitingTrigger` component)
#[derive(Component, Clone)]
#[require(BehaveTargetEntity)]
#[require(Name::new("BehaveTree"))]
pub struct BehaveTree {
    tree: Tree<BehaveNode>,
    logging: bool,
}
impl std::fmt::Display for BehaveTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        walk_tree(self.tree.root(), 0, f)?;
        Ok(())
    }
}

fn walk_tree(
    node: NodeRef<BehaveNode>,
    depth: usize,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    for _ in 0..(depth * 2) {
        write!(f, " ")?;
    }
    write!(f, "* ")?;
    writeln!(f, "{}  [{:?}]", node.value(), node.id())?;
    for child in node.children() {
        walk_tree(child, depth + 1, f)?;
    }
    Ok(())
}

/// Verifies that nodes have the appropriate number of children etc.
fn verify_tree(node: &NodeRef<Behave>) -> bool {
    let children = node.children().collect::<Vec<_>>();
    let n = node.value();
    let range = n.permitted_children();
    if !range.contains(&children.len()) {
        error!(
            "⁉️  Node {n} has {} children! Valid range is: {range:?}",
            children.len(),
        );
        false
    } else {
        for child in children.iter() {
            if !verify_tree(child) {
                return false;
            }
        }
        true
    }
}

impl TickCtx {
    /// Create a new TickCtx with the given behaviour tree entity and target entity.
    pub(crate) fn new(bt_entity: Entity, target_entity: Entity, elapsed_secs: f32) -> Self {
        Self {
            bt_entity,
            target_entity,
            supervisor_entity: None,
            elapsed_secs,
            logging: false,
        }
    }
    /// Set the optional supervisor entity that is controlling the behaviour tree.
    /// This is only used when running under my unreleased HTN crate that complements bevy_behave.
    pub(crate) fn with_optional_sup_entity(mut self, sup_entity: Option<Entity>) -> Self {
        self.supervisor_entity = sup_entity;
        self
    }

    #[allow(unused)]
    pub(crate) fn with_logging(mut self, logging: bool) -> Self {
        self.logging = logging;
        self
    }
}

/// Context passed down the recursive tree ticking fn
#[derive(Debug)]
pub(crate) struct TickCtx {
    /// Enable for verbose logging (for debugging, too verbose for production)
    #[allow(unused)]
    pub(crate) logging: bool,
    /// The entity of the behaviour tree.
    pub(crate) bt_entity: Entity,
    /// The entity of the target character the tree is controlling..
    pub(crate) target_entity: Entity,
    /// The entity of the tree supervisor (if present).
    /// This is not used by bevy_behave unless the tree is running under my complementary
    /// HTN crate for planning, which is not yet released.
    pub(crate) supervisor_entity: Option<Entity>,
    /// Bevy's Time res elapsed_secs
    pub(crate) elapsed_secs: f32,
}

impl BehaveTree {
    /// Creates a BehaveTree from an `ego_tree::Tree<BehaveNode>`.
    /// Typically this is created using the behave! macro, but can be
    /// constructed using the ego_tree api too.
    ///
    /// # Panics
    /// An invalid tree will cause a panic here.
    /// Use BehaveTree::verify(&tree) to verify your tree definition first.
    pub fn new(tree: Tree<Behave>) -> Self {
        if !Self::verify(&tree) {
            panic!("Invalid tree");
        }
        // convert to internal BehaveNode tree
        let tree = tree.map(BehaveNode::new);
        Self {
            tree,
            logging: false,
        }
    }

    /// Checks the tree definition is valid by verifying that each node has the correct
    /// number of children.
    pub fn verify(tree: &Tree<Behave>) -> bool {
        verify_tree(&tree.root())
    }

    /// Should verbose logging be enabled? (typically just for debugging).
    pub fn with_logging(mut self, enabled: bool) -> Self {
        self.logging = enabled;
        self
    }

    fn tick(&mut self, commands: &mut Commands, tick_ctx: &TickCtx) -> BehaveNodeStatus {
        let mut node = self.tree.root_mut();
        tick_node(&mut node, commands, tick_ctx)
    }

    /// Returns Option<Entity> being an entity that was spawned to run this task node.
    /// (so it can be despawned now that the task is complete)
    /// Will always be none if reporting a result from a TriggerReq node.
    pub(crate) fn set_node_result(&mut self, ctx: &BehaveCtx, success: bool) -> Option<Entity> {
        let node_id = ctx.task_node();
        let mut node = self.tree.get_mut(node_id).unwrap();
        let val = node.value();
        match val {
            BehaveNode::DynamicEntity { task_status, .. } if ctx.is_for_entity() => {
                // extract the entity that was running this node, so we can despawn it
                let task_entity = match task_status {
                    EntityTaskStatus::Started(e) => Some(*e),
                    _ => {
                        warn!("Given node ({node_id:?}) result for a non-spawned entity node?");
                        None
                    }
                };
                if self.logging {
                    debug!(
                        "Setting Dynamic Entity task for {node_id:?} success to {:?}",
                        success
                    );
                }
                *task_status = EntityTaskStatus::Complete(success);
                task_entity
            }
            BehaveNode::TriggerReq { task_status, .. } => {
                if self.logging {
                    debug!(
                        "Setting conditional task for {node_id:?} success to {:?}",
                        success
                    );
                }
                *task_status = TriggerTaskStatus::Complete(success);
                None
            }
            _ => {
                error!("Given node result but no matching node found: {node_id:?}");
                None
            }
        }
    }
}

/// Will report success or failure after a timeout
#[derive(Component, Debug, Clone, Reflect)]
pub struct BehaveTimeout {
    duration: std::time::Duration,
    should_succeed: bool,
    start_time: f32,
}

impl BehaveTimeout {
    /// Creates a new BehaveTimeout which will trigger success or failure after a given duration.
    pub fn new(duration: std::time::Duration, should_succeed: bool) -> Self {
        Self {
            duration,
            should_succeed,
            start_time: 0.0,
        }
    }
    /// Creates a new BehaveTimeout which will trigger success or failure after a given number of seconds
    pub fn from_secs(secs: f32, should_succeed: bool) -> Self {
        Self::new(std::time::Duration::from_secs(secs as u64), should_succeed)
    }
}

fn on_tick_timeout_added(
    t: On<Add, BehaveTimeout>,
    mut q: Query<&mut BehaveTimeout>,
    time: Res<Time>,
) {
    let mut timeout = q.get_mut(t.event().entity).unwrap();
    timeout.start_time = time.elapsed_secs();
}

fn tick_timeout_components(
    q: Query<(&BehaveTimeout, &BehaveCtx)>,
    time: Res<Time>,
    mut commands: Commands,
) {
    for (timeout, ctx) in q.iter() {
        let elapsed = time.elapsed_secs() - timeout.start_time;
        if elapsed >= timeout.duration.as_secs_f32() {
            if timeout.should_succeed {
                commands.trigger(ctx.success());
            } else {
                commands.trigger(ctx.failure());
            }
        }
    }
}
