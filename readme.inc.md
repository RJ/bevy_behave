<div align="left">
<p>
    <strong>A behaviour tree plugin for bevy with dynamic spawning.</strong>
</p>
<p>
    <a href="https://crates.io/crates/bevy_behave"><img src="https://img.shields.io/crates/v/bevy_behave.svg" alt="crates.io"/></a>
    <a href="https://docs.rs/bevy_behave"><img src="https://img.shields.io/badge/docs-latest-blue.svg" alt="docs.rs"/></a>
    <a href="https://discord.com/channels/691052431525675048/1347180005104422942"><img src="https://img.shields.io/badge/discord-bevy_behave-blue" alt="discord channel"/></a>
    
</p>
</div>

`bevy_behave` is a behaviour tree plugin for bevy with a sensible API and minimal overheads.
No magic is required for the task components, they are are regular bevy components using triggers to report status.

When an action node (aka leaf node or task node) in the behaviour tree runs, it will spawn an entity with
the components you specified in the tree definition. The tree then waits for this entity to
trigger a status report, at which point the entity will be despawned.

You can also take actions without spawning an entity by triggering an observed `Event`, which can also be used as a conditional in a control node.


This tree definition is from the [chase example](https://github.com/RJ/bevy_behave/blob/main/examples/chase.rs):

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# fn get_enemy_entity() -> Entity { Entity::PLACEHOLDER }
# fn get_player_entity() -> Entity { Entity::PLACEHOLDER }
# #[derive(Component, Clone)]
# struct WaitUntilPlayerIsNear { player: Entity }
# #[derive(Component, Clone)]
# struct MoveTowardsPlayer { player: Entity, speed: f32 }
# #[derive(Clone)]
# struct RandomizeColour;
let npc_entity = get_enemy_entity();
let player = get_player_entity();
// The tree definition (which is cloneable).
// and in theory, able to be loaded from an asset file using reflection (PRs welcome).
// When added to the BehaveTree component, this gets transformed internally to hold state etc.
//
// These trees are `ego_tree::Tree<Behave>` if you want to construct them manually.
// Conventient macro usage shown below.
let tree = behave! {
    Behave::Forever => {
        Behave::Sequence => {
            Behave::spawn((
                Name::new("Wait until player is near"),
                WaitUntilPlayerIsNear{player}
            )),
            Behave::Sequence => {
                Behave::spawn((
                    Name::new("Move towards player while in range"),
                    MoveTowardsPlayer{player, speed: 100.0}
                )),
                // MoveTowardsPlayer suceeds if we catch them, in which randomize our colour.
                // This uses a trigger to take an action without spawning an entity.
                Behave::trigger(RandomizeColour),
                // then have a nap (pause execution of the tree)
                // NB: this only runs if the trigger_req was successful, since it's in a Sequence.
                Behave::Wait(5.0),
            }
        }
    }
};
```


<details>

<summary><small>You can also compose trees from subtrees</small></summary>

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# fn get_enemy_entity() -> Entity { Entity::PLACEHOLDER }
# fn get_player_entity() -> Entity { Entity::PLACEHOLDER }
# #[derive(Component, Clone)]
# struct WaitUntilPlayerIsNear { player: Entity }
# #[derive(Component, Clone)]
# struct MoveTowardsPlayer { player: Entity, speed: f32 }
# #[derive(Clone)]
# struct RandomizeColour;

let npc_entity = get_enemy_entity();
let player = get_player_entity();
// Breaking a tree into two trees and composing, just to show how it's done.
let chase_subtree = behave! {
    Behave::Sequence => {
        Behave::spawn((
            Name::new("Move towards player while in range"),
            MoveTowardsPlayer{player, speed: 100.0}
        )),
        // MoveTowardsPlayer suceeds if we catch them, in which randomize our colour.
        // This uses a trigger to take an action without spawning an entity.
        Behave::trigger(RandomizeColour),
        // then have a nap (pause execution of the tree)
        // NB: this only runs if the trigger_req was successful, since it's in a Sequence.
        Behave::Wait(5.0),
    }
};

let tree = behave! {
    Behave::Forever => {
        // Run children in sequence until one fails
        Behave::Sequence => {
            // WAIT FOR THE PLAYER TO GET CLOSE
            // Spawn with any normal components that will control the target entity:
            Behave::spawn((
                Name::new("Wait until player is near"),
                WaitUntilPlayerIsNear{player}
            )),
            // CHASE THE PLAYER
            @ chase_subtree
        }
    }
};
```

</details>

<br>

Once you have your tree definition, you spawn an entity to run the behaviour tree by adding a `BehaveTree` component:

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# fn setup_tree(mut commands: Commands, tree: ego_tree::Tree<Behave>, npc_entity: Entity) {
// Spawn an entity to run the behaviour tree.
// Make it a child of the npc entity for convenience.
// The default is to assume the Parent of the tree entity is the Target Entity you're controlling.
commands.spawn((
    Name::new("Behave tree for NPC"),
    BehaveTree::new(tree),
    ChildOf(npc_entity),
));
# }
```

If your behaviour tree is not a child of the target entity you want to control, you can specify the target entity explicitly:

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# fn get_entity_to_control() -> Entity { Entity::PLACEHOLDER }
# fn setup_tree(mut commands: Commands, tree: ego_tree::Tree<Behave>) {
let target = get_entity_to_control();
commands.spawn((
    Name::new("Behave tree for NPC"),
    BehaveTree::new(tree),
    BehaveTargetEntity::Entity(target),
));
# }
```

Or in case of a deeper hierarchy, you can use `BehaveTargetEntity::RootAncestor` to find the topmost entity.



## Control Flow Nodes

The following control flow nodes are supported. Control flow logic is part of the `BehaveTree` and doesn't spawn extra entities.

| Node                    | Description                                                                                                                       |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `Behave::Sequence`      | Runs children in sequence, failing if any child fails, succeeding if all children succeed.                                        |
| `Behave::Fallback`      | Runs children in sequence until one succeeds. If all fail, this fails. Sometimes called a Selector node.                          |
| `Behave::Invert`        | Inverts success/failure of child. Must only have one child.                                                                       |
| `Behave::AlwaysSucceed` | Succeeds instantly.                                                                                                               |
| `Behave::AlwaysFail`    | Fails instantly.                                                                                                                  |
| `Behave::While`         | Runs the second child repeatedly, provided the first child returns success. If only one child, runs it repeatedly until it fails. |
| `Behave::IfThen`        | If the first child succeeds, run the second child. (otherwise, run the optional third child)                                      |


### Control Flow Node Examples


#### Sequence

Use `Behave::Sequence` to run children in sequence, failing if any child fails, succeeding if all children succeed.

This example runs a trigger (and assuming it reports success..), waits 5 secs, then spawns an entity with an imagined `BTaskComponent` to do something.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Component, Default, Clone)]
# struct BTaskComponent;
# #[derive(Clone)]
# struct DoA;
let tree = behave! {
    Behave::Sequence => {
        Behave::trigger(DoA),
        Behave::Wait(5.0),
        Behave::spawn_named("B-Doer", BTaskComponent::default()),
    }
};
```


#### Fallback

Use `Behave::Fallback` to run children in sequence until one succeeds. If they all fail, the Fallback node also fails.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone)]
# struct TryA;
# #[derive(Clone)]
# struct TryB;
# #[derive(Clone)]
# struct TryC;
let tree = behave! {
    Behave::Fallback => {
        Behave::trigger(TryA),
        Behave::trigger(TryB),
        Behave::trigger(TryC),
    }
};
```

#### While (single child usage)

You can wrap a single node in a `Behave::While` node to repeat it until it fails.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone)]
# struct DoSlowThingUntilFailure;
let tree = behave! {
    Behave::While => {
        Behave::trigger(DoSlowThingUntilFailure),
    }
};
```

#### While (two child usage)

With two children, the first child is the conditional check. If it succeeds, the second child is run. And then the node repeats.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone)]
# struct AirbourneCheck;
# #[derive(Clone, Component, Default)]
# struct FlapWings;
# #[derive(Clone, Component, Default)]
# struct PointToes;
let tree = behave! {
    Behave::While => {
        Behave::trigger(AirbourneCheck),
        Behave::spawn_named("Fly!", (FlapWings::default(), PointToes::default())),
    }
};
```

#### IfThen (two child usage)

The first child is the conditional check, the second is only run if the condition succeeds.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone)]
# struct HungryCheck;
# #[derive(Clone, Component, Default)]
# struct MoveToFood;
# #[derive(Clone, Default)]
# struct EatFood;
let tree = behave! {
    Behave::IfThen => {
        Behave::trigger(HungryCheck),
        Behave::Sequence => {
            // move to food, but only allow 10 seconds to do so. Then eat, if we got there.
            Behave::spawn_named("Go to food", (MoveToFood::default(), BehaveTimeout::from_secs(10.0, false))),
            Behave::trigger(EatFood),
        },
    }
};
```

#### IfThen (three child usage)

An optional third child acts as the "else" clause, and is run if the conditional fails.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone)]
# struct HungryCheck;
# #[derive(Clone, Component, Default)]
# struct MoveToFood;
# #[derive(Clone, Default)]
# struct EatFood;
# #[derive(Clone)]
# struct TidyKitchen;
let tree = behave! {
    Behave::IfThen => {
        Behave::trigger(HungryCheck),
        Behave::Sequence => {
            Behave::spawn_named("Go to food", (MoveToFood::default(), BehaveTimeout::from_secs(10.0, false))),
            Behave::trigger(EatFood),
        },
        Behave::trigger(TidyKitchen),
    }
};
```


## Task Nodes

Task nodes are leaves of the tree which take some action, typically doing something to control your target entity, such as making it move.

#### Behave::Wait

Waits a given duration before Succeeding. The timer is ticked by the tree itself, so no entities are spawned.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
let tree = behave! {
    Behave::Wait(5.0),
};
```

#### Behave::spawn(...) and Behave::spawn_named(...)

When a `Behave::spawn_named` node runs, a new entity is spawned with the bundle of components you provided along with a
`BehaveCtx` component, used to get the target entity the tree is controlling, and the mechanism to generate status reports.

Once a result is reported, the entity is despawned.

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone, Component, Default)]
# struct WingFlapper;
// Flap our wings, and succeed (end the task) after 60 seconds.
let tree = behave! {
    Behave::spawn_named("Flying Task", 
        (WingFlapper::default(), BehaveTimeout::from_secs(60.0, true))
    )
};
```

Prefer the `Behave::spawn_named` variant, because in addition to adding a `Name` component to the spawned entity, it exposes this name in debug logging.

<details>

<summary>An example implementation (click to reveal)</summary>

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Component, Clone, Default)]
# struct Wings;
# impl Wings { fn flap(&mut self, speed: f32) { } }
# #[derive(Component, Clone, Default)]
# struct BirdMarker;
// An example plugin to provide a `WingFlapper` task component.

fn wing_flapper_task_plugin(app: &mut App) {
    app.add_systems(FixedUpdate, wing_flap_system);
}

#[derive(Component, Clone, Default)]
struct WingFlapper {
    speed: f32,
}

fn wing_flap_system(
    mut q_target: Query<&mut Wings, With<BirdMarker>>,
    flapper_tasks: Query<(&WingFlapper, &BehaveCtx)>,
    mut commands: Commands
) {
    // for each entity with a WingFlapper component and a BehaveCtx, flap the wings for its target entity
    for (flapper, ctx) in flapper_tasks.iter() {
        // the target entity is the one being controlled by the behaviour tree that spawned this task entity
        let target = ctx.target_entity();
        let Ok(mut target_wings) = q_target.get_mut(target) else {
            // Maybe the wings fell off? report task failure.
            commands.trigger(ctx.failure());
            continue;
        };
        target_wings.flap(flapper.speed);
    }
}
```
</details>


#### Behave::trigger(...)

When a `Behave::trigger` node runs, it will trigger an event, which the user observes and can either respond to with a success or failure immediately, or respond later from another system. You must specify an arbitrary `Clone` type which is passed along as
the payload of the trigger event, along with the `BehaveCtx`.

Here's how you might use a trigger conditional check to execute a specific task if a height condition is met:

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone)]
# struct HeightCheck { min_height: f32 }
# #[derive(Clone, Component, Default)]
# struct TakeActionWhenHigh;
let tree = behave! {
    Behave::IfThen => {
        Behave::trigger(HeightCheck { min_height: 10.0 }),
        Behave::spawn_named("High Thing", TakeActionWhenHigh::default()),
    }
};
```
<details>

<summary>And the implementation (click to reveal)</summary>


```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone, Component)]
# struct Position { x: f32, y: f32 }
// An example plugin to provide a `HeightCheck` trigger task

fn height_check_task_plugin(app: &mut App) {
    // add a global observer to answer conditional queries for HeightCheck:
    app.add_observer(on_height_check);
}

// Trigger payloads just need to be Clone.
// They are wrapped in a BehaveTrigger, which is a bevy Event.
#[derive(Clone)]
struct HeightCheck {
    min_height: f32,
}

// you respond by triggering a success or failure event created by the ctx:
fn on_height_check(trigger: On<BehaveTrigger<HeightCheck>>, q: Query<&Position>, mut commands: Commands) {
    let ev = trigger.event();
    let ctx: &BehaveCtx = ev.ctx();
    let height_check: &HeightCheck = ev.inner();
    // lookup the position of the target entity (ie the entity this behaviour tree is controlling)
    let character_pos = q.get(ctx.target_entity()).expect("Character entity missing?");
    if character_pos.y >= height_check.min_height {
        commands.trigger(ctx.success());
    } else {
        commands.trigger(ctx.failure());
    }
}

```
</details>

<br>

If you respond with a success or failure from the observer you can treat the event as a conditional test as part of a control flow node. Alternatively, you can use it to trigger a side effect and respond later from another system. Just make sure to copy the `BehaveCtx` so you can generate a success or failure event at your leisure.



## Cargo Example

Have a look at the [chase example](https://github.com/RJ/bevy_behave/blob/main/examples/chase.rs) to see how these are used.
Run in release mode to support 100k+ enemies at once:
```bash
cargo run --release --example chase
```


## Utility components

For your convenience:

#### Triggering completion after a timeout

To trigger a status report on a dynamic spawn task after a timeout, use the `BehaveTimeout` helper component:

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# #[derive(Clone, Component, Default)]
# struct LongRunningTaskComp;
let tree = behave! {
    Behave::spawn_named("Long running task that succeeds after 5 seconds", (
        LongRunningTaskComp::default(),
        BehaveTimeout::from_secs(5.0, true)
    ))
};
```

This will get the `BehaveCtx` from the entity, and trigger a success or failure report for you after the timeout.



## `behave!` macro

The `behave!` macro is more powerful version of the `ego_tree::tree!` macro.
You can use ego_tree's `tree!` macro to build the tree, but this macro has some additional features
to make composing behaviours easier:

#### Merging in subtrees:

Use `@` to insert a subtree into the current tree:
```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
#[derive(Clone)]
struct A;
#[derive(Clone)]
struct B;
fn get_tree() -> Tree<Behave> {
    let subtree = behave! {
        Behave::Sequence => {
            Behave::trigger(A),
            Behave::Wait(1.0),
            Behave::trigger(B),
        }
    };

    behave! {
        Behave::Sequence => {
            Behave::Wait(5.0),
            @ subtree
        }
    }
}
```

Use `...` to insert multiple subtrees from an iterator of trees:
```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
#[derive(Clone)]
struct A;
#[derive(Clone)]
struct B;
fn get_tree() -> Tree<Behave> {
    let subtrees = [
        behave! { Behave::Wait(1.0) },
        behave! { Behave::Wait(2.0) },
        behave! { Behave::Wait(3.0) },
    ];

    behave! {
        Behave::Sequence => {
            Behave::Wait(5.0),
            ... subtrees
        }
    }
}
```


#### Inserting nodes from an iterator:

Use `@[ ]` to insert leaf nodes (`Behave` enum type, not a tree) from an iterator:
```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
#[derive(Clone)]
struct A;
#[derive(Clone)]
struct B;
fn get_tree() -> Tree<Behave> {
    let children = vec![
        Behave::trigger(A),
        Behave::Wait(1.0),
        Behave::trigger(B),
    ];
    behave! {
        Behave::Sequence => {
            @[ children ]
        }
    }
}
```

## Debug Logging

Call `BehaveTree::with_logging(true)` to enable debug verbose logging:

```rust
# use bevy_behave::prelude::*;
# use bevy::prelude::*;
# fn setup_tree(mut commands: Commands) {

let tree = behave! { Behave::Wait(5.0) }; // etc

commands.spawn((
    Name::new("Behave tree for NPC"),
    BehaveTree::new(tree).with_logging(true),
));
# }
```

<img src="https://github.com/RJ/bevy_behave/blob/main/examples/console_logging.png">

## Performance

is good.

* There's just one global observer for receiving task status reports from entities or triggers.
* Most of the time, the work is being done in a spawned entity using one of your action components,
and in this state, there is a marker on the tree entity so it doesn't tick or do anything until
a result is ready.
* Avoided mut World systems – the tree ticking should be able to run in parallel with other things.
* So a fairly minimal wrapper around basic bevy systems.

In release mode, i can happily toss 100k enemies in the chase demo and zoom around at max framerate.
It gets slow rendering a zillion gizmo circles before any bevy_behave stuff gets in the way.

**Chase example**

This is the chase example from this repo, running in release mode on an M1 mac with 100k enemies.
Each enemy has a behaviour tree child and an active task component entity. So 1 enemy is 3 entities.

https://github.com/user-attachments/assets/e12bc4dd-d7fb-4eca-8810-90d65300776d

**Video from my space game**

Here I have more complex behaviour trees managing orbits, landing, etc. Lots of PID controllers at work.
No attempts at optimising the logic yet, but I can add 5k ships running behaviours. Each is a dynamic avian physics object exerting forces via a thruster.




https://github.com/user-attachments/assets/ef4f0539-0b4d-4d57-9516-a39783de140f


## Bevy Version Compatibility

| bevy_behave | bevy |
| ----------- | ---- |
| 0.5         | 0.18 |
| 0.4         | 0.17 |
| 0.3         | 0.16 |
| 0.2.2       | 0.15 |


## Chat / Questions?

Say hi in the [bevy_behave discord channel](https://discord.com/channels/691052431525675048/1347180005104422942).

## Further Reading

* Cool interactive blog post using bevy_behave: https://www.hankruiger.com/posts/bevy-behave/
* [Wikipedia on Behavior Trees](https://en.wikipedia.org/wiki/Behavior_tree_(artificial_intelligence,_robotics_and_control))


## License

Same as bevy: MIT or Apache-2.0.

<hr>


#### Paths not taken

<details>

<summary>Alternative approach taking `IntoSystem` (not taken)</summary>

### Alternative approach for conditionals

I considered doing control flow by taking an `IntoSystem` with a defined In and Out type,
something like this:
```rust,ignore

pub type BoxedConditionSystem = Box<dyn System<In = In<BehaveCtx>, Out = bool>>;

#[derive(Debug)]
pub enum Behave {
    // ...
    /// If, then
    Conditional(BoxedConditionSystem),
}

impl Behave {
    pub fn conditional<Marker>(system: impl IntoSystem<In<BehaveCtx>, bool, Marker>) -> Behave {
        Behave::Conditional(Box::new(IntoSystem::into_system(system)))
    }
}
```

Then you could defined a cond system like, which is quite convenient:

```rust,ignore
fn check_distance(In(ctx): In<BehaveCtx>, q: Query<&Position, With<Player>>) -> bool {
    let Ok(player_pos) = q.get(ctx.target_entity).unwrap();
    player_pos.x < 100.0
}
```


However I don't think the resulting data struct would be cloneable, nor could you really read
it from an asset file for manipulation (or can you?)

I would also need mutable World in the "tick trees" system, which would stop it running in parallel maybe.
Anyway observers seem to work pretty well.
</details>

