/// Example demonstrating BehaveInterrupt usage
///
/// This example shows how to use BehaveInterrupt to interrupt long-running tasks
/// based on trigger conditions. A mining task will be interrupted if:
/// - Player health drops below 30%
/// - An enemy appears nearby
///
/// The example simulates a world where:
/// - Player health decreases over time
/// - An enemy appears with 20% chance
/// - The mining task never stops
///
/// You'll see the mining task get interrupted when conditions change.
use bevy::{log::LogPlugin, prelude::*};
use bevy_behave::prelude::*;

/// ask BehaveTree to log transitions
const ENABLE_LOGGING: bool = true;

fn main() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(LogPlugin::default());
    app.add_plugins(BehavePlugin::default());
    app.add_systems(Startup, init);
    app.add_observer(on_check_player_healthy);
    app.add_observer(on_check_enemy_nearby);
    app.add_observer(on_task_finished);
    app.add_systems(FixedUpdate, simulate_world_changes);
    app.run();
}

#[derive(Component, Clone)]
struct MiningTask;

#[derive(Clone)]
struct CheckPlayerHealthy;

#[derive(Clone)]
struct CheckEnemyNearby;

#[derive(Resource)]
struct GameState {
    player_health: f32,
    enemy_nearby: bool,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            player_health: 100.,
            enemy_nearby: false,
        }
    }
}

fn on_check_player_healthy(
    trigger: Trigger<BehaveTrigger<CheckPlayerHealthy>>,
    mut commands: Commands,
    game_state: Res<GameState>,
) {
    if game_state.player_health > 30.0 {
        commands.trigger(trigger.ctx().success());
    } else {
        commands.trigger(trigger.ctx().failure());
    }
}

fn on_check_enemy_nearby(
    trigger: Trigger<BehaveTrigger<CheckEnemyNearby>>,
    mut commands: Commands,
    game_state: Res<GameState>,
) {
    if game_state.enemy_nearby {
        commands.trigger(trigger.ctx().success());
    } else {
        commands.trigger(trigger.ctx().failure());
    }
}

fn on_task_finished(_trigger: Trigger<OnAdd, BehaveFinished>, mut exit: EventWriter<AppExit>) {
    exit.write(AppExit::Success);
}

fn simulate_world_changes(mut game_state: ResMut<GameState>) {
    game_state.player_health -= 15.0_f32.max(0.0);
    info!("New health: {}", game_state.player_health);

    let enemy_appearing = rand::random::<f32>() < 0.2;
    if enemy_appearing {
        info!("Spawning Enemy!");
        game_state.enemy_nearby = true;
    }
}

fn init(mut commands: Commands) {
    // Create a behavior tree that attempts to mine resources
    // but can be interrupted by low health or nearby enemies
    let tree = behave! {
        Behave::Sequence => {
            Behave::spawn_named("Mining with interrupts", (
                MiningTask,
                BehaveInterrupt::by(CheckEnemyNearby).or_not(CheckPlayerHealthy),
                BehaveTimeout::from_secs(10., false)
            )),
        }
    };

    let target = commands.spawn_empty().id();

    commands.spawn((
        Name::new("Behavior Tree"),
        BehaveTree::new(tree).with_logging(ENABLE_LOGGING),
        BehaveTargetEntity::Entity(target),
    ));

    commands.init_resource::<GameState>();
}

