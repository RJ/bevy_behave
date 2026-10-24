// This is a a trigger version of an updated version of https://crates.io/crates/bevy_dynamic_bundle
use crate::ctx::BehaveCtx;
use bevy::prelude::*;
use dyn_clone::DynClone;

/// A wrapper around a user-provided type, which we trigger to test a condition or cause an action.
#[derive(Event, Debug, Clone)]
pub struct BehaveTrigger<T: Clone + Send + Sync> {
    pub(crate) inner: T,
    pub(crate) ctx: BehaveCtx,
}

impl<T: Clone + Send + Sync> BehaveTrigger<T> {
    /// Returns the context for this trigger event.
    pub fn ctx(&self) -> &BehaveCtx {
        &self.ctx
    }
    /// Returns the inner value of the trigger, typically the struct that was
    /// passed to `Behave::trigger_req`.
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

fn world_trigger<T: Clone + Send + Sync + 'static>(bundle: T) -> impl DynTriggerCommand {
    move |ctx: BehaveCtx, world: &mut World| {
        let ev = BehaveTrigger::<T> { inner: bundle, ctx };
        world.trigger(ev);
    }
}

trait DynTriggerCommand<Marker = ()>: DynClone + Send + Sync + 'static {
    fn apply(self: Box<Self>, ctx: BehaveCtx, world: &mut World);
}

impl<F> DynTriggerCommand for F
where
    F: FnOnce(BehaveCtx, &mut World) + DynClone + Send + Sync + 'static,
{
    fn apply(self: Box<Self>, ctx: BehaveCtx, world: &mut World) {
        self(ctx, world);
    }
}

struct CommandWrapper {
    ctx: BehaveCtx,
    cmd: DynamicTrigger,
}

impl Command for CommandWrapper {
    type Out = ();
    fn apply(self, world: &mut World) -> Self::Out {
        self.cmd.trig_fn.apply(self.ctx, world);
    }
}

dyn_clone::clone_trait_object!(DynTriggerCommand);
#[derive(Clone)]
pub struct DynamicTrigger {
    #[allow(dead_code)]
    trig_fn: Box<dyn DynTriggerCommand>,
    type_name: String,
}
impl std::fmt::Debug for DynamicTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DynamicTrigger({})", self.type_name)
    }
}

impl DynamicTrigger {
    pub fn type_name(&self) -> &str {
        &self.type_name
    }
    pub fn new<T: Clone + Send + Sync + 'static>(trig: T) -> DynamicTrigger {
        DynamicTrigger {
            trig_fn: Box::new(world_trigger(trig)),
            // preserve the type name for debugging
            type_name: std::any::type_name::<T>().to_string(),
        }
    }
}

#[allow(dead_code)]
pub trait DynamicTriggerCommand {
    fn dyn_trigger(&mut self, dyn_trigger: DynamicTrigger, ctx: BehaveCtx);
}

// Implementation for Commands
impl DynamicTriggerCommand for Commands<'_, '_> {
    fn dyn_trigger(&mut self, dyn_trigger: DynamicTrigger, ctx: BehaveCtx) {
        self.queue(CommandWrapper {
            ctx,
            cmd: dyn_trigger,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct MyStruct(u32);

    #[derive(Resource, Default)]
    struct TrigRes {
        val: Option<MyStruct>,
    }

    fn on_trig(trigger: On<BehaveTrigger<MyStruct>>, mut res: ResMut<TrigRes>) {
        res.val = Some(*trigger.event().inner());
    }

    fn send_trigger(mut commands: Commands) {
        let dyn_trig = DynamicTrigger::new(MyStruct(123));
        let tick_ctx = crate::TickCtx::new(Entity::PLACEHOLDER, Entity::PLACEHOLDER, 0.0);
        let ctx = BehaveCtx::new_for_entity(get_node_id(), &tick_ctx, Entity::PLACEHOLDER);
        commands.dyn_trigger(dyn_trig, ctx);
    }

    #[test]
    fn dyn_trigger_test() {
        let mut app = App::new();
        app.init_resource::<TrigRes>()
            .add_observer(on_trig)
            .add_systems(Startup, send_trigger);
        app.update();
        assert_eq!(app.world().resource::<TrigRes>().val, Some(MyStruct(123)));
    }

    fn get_node_id() -> ego_tree::NodeId {
        let t = ego_tree::tree! { 1 };
        t.root().id()
    }
}
