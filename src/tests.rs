// NB: you can println!("{}", tree); and run the test like this to see output:
// cargo test -- --nocapture test_at_list
use crate::prelude::*;

/// Empty sequences are permitted.
#[test]
fn test_empty_control_flow() {
    let tree = behave! {
        Behave::Sequence => {
            @[[]]
        }
    };
    assert!(BehaveTree::verify(&tree));
    assert_tree("Sequence", tree);

    let tree = behave! {
        Behave::Fallback => {
        }
    };
    assert!(BehaveTree::verify(&tree));
    assert_tree("Fallback", tree);
}

/// Tests using the @ [] syntax for including a list of task nodes,
/// eg Behave::spawn_named or Wait etc – nothing that has children.
#[test]
fn test_at_list() {
    let behaviours = [Behave::Wait(1.0), Behave::Wait(2.0), Behave::Wait(3.0)];
    let tree = behave! {
        Behave::Sequence => {
            Behave::Wait(5.0),
            @[ behaviours ]
        }
    };
    assert_tree(
        "Sequence
            ├── Wait(5s)
            ├── Wait(1s)
            ├── Wait(2s)
            └── Wait(3s)",
        tree,
    );
}

/// Tests using the @ syntax to insert a single subtree
#[test]
fn test_at_tree() {
    let subtree = behave! {
        Behave::Sequence => {
            Behave::Wait(1.0),
            Behave::Wait(2.0),
        }
    };
    let tree = behave! {
        Behave::Sequence => {
            Behave::Wait(5.0),
            @ subtree
        }
    };
    assert_tree(
        "Sequence
            ├── Wait(5s)
            └── Sequence
                ├── Wait(1s)
                └── Wait(2s)",
        tree,
    );
}

/// Shows how to use the ego_tree API to build a tree,
/// and then shows how to use the `...` syntax to append a list of subtrees.
#[test]
fn test_ego_tree_api() {
    let trees = [
        behave! {
            Behave::Wait(1.0),
        },
        behave! {
            Behave::Sequence => {
                Behave::Wait(1.0),
                Behave::Wait(2.0),
            }
        },
    ];
    let mut tree = ego_tree::Tree::new(Behave::Sequence);
    let mut root = tree.root_mut();
    root.append(Behave::Wait(0.1));
    for subtree in trees.clone() {
        root.append_subtree(subtree);
    }
    assert_tree(
        "Sequence
            ├── Wait(0.1s)
            ├── Wait(1s)
            └── Sequence
                ├── Wait(1s)
                └── Wait(2s)",
        tree.clone(),
    );

    // the ... syntax appends a list of subtrees, so this creates the same tree as above:
    let t2 = behave! {
        Behave::Sequence => {
            Behave::Wait(0.1),
            ... trees,
        }
    };

    assert_eq!(tree.to_string(), t2.to_string());
}

#[test]
fn test_root_ancestor_with_nested_trees() {
    use crate::prelude::*;
    use bevy::prelude::*;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(BehavePlugin::default());
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_systems(Startup, |mut commands: Commands| {
        let tree = behave! {
            Behave::Sequence => {
                Behave::Wait(1.0),
                Behave::spawn_named("Nested tree", BehaveTree::new(behave! { Behave::Wait(2.0) }).with_logging(true)),
            }
        };
        let id = commands.spawn(BehaveTree::new(tree).with_logging(true)).id();
        info!("spawned tree with id: {}", id);
    });
    app.add_observer(
        |t: Trigger<OnAdd, BehaveFinished>,
         q: Query<(&BehaveFinished, &BehaveCtx)>,
         mut exit: EventWriter<AppExit>,
         mut commands: Commands| {
            let Ok((finished, ctx)) = q.get(t.target()) else {
                // if there was no BehaveCtx on this entity, it was the topmost tree, so exit the test
                exit.write(AppExit::Success);
                return;
            };
            if finished.0 {
                commands.trigger(ctx.success());
            } else {
                commands.trigger(ctx.failure());
            }
        },
    );
    app.run();
}

#[test]
fn test_frame_delays_async() {
    run_frame_delays(false, 5);
}

#[test]
fn test_frame_delays_sync() {
    run_frame_delays(true, 1);
}

#[test]
fn test_while_node() {
    use crate::prelude::*;
    use bevy::prelude::*;

    #[derive(Event, Clone)]
    struct RunAssert;

    #[derive(Event, Clone)]
    struct Count;

    fn should_only_run_once(
        trigger: Trigger<BehaveTrigger<RunAssert>>,
        mut cmd: Commands,
        mut count: Local<u32>,
    ) {
        *count += 1;
        assert_eq!(*count, 1);
        cmd.trigger(trigger.ctx.success());
    }

    fn run_twice(
        trigger: Trigger<BehaveTrigger<Count>>,
        mut cmd: Commands,
        mut count: Local<u32>,
        mut exit: EventWriter<AppExit>,
    ) {
        *count += 1;
        cmd.trigger(trigger.ctx.success());
        if *count == 2 {
            exit.write(AppExit::Success);
        }
    }

    let mut app = App::new();
    let app = app
        .add_plugins((
            BehavePlugin::default(),
            MinimalPlugins,
            bevy::log::LogPlugin::default(),
        ))
        .add_observer(should_only_run_once)
        .add_observer(run_twice)
        .add_systems(Startup, |mut cmd: Commands| {
            let tree = behave! {
                Behave::Forever => {
                    Behave::Sequence => {
                        Behave::trigger(RunAssert),
                        Behave::While => {
                            Behave::AlwaysSucceed,
                            Behave::trigger(Count),
                        }
                    }
                }
            };
            cmd.spawn(BehaveTree::new(tree).with_logging(true));
        });
    app.run();
}

/// Increments a u32 frame counter at the start of FixedPreUpdate, before the trees tick.
/// Checks that the final frame number matches what we expect, once the topmost tree finishes.
///
/// Toggle for async/sync ticking.
fn run_frame_delays(sync: bool, expected_final_frame: u32) {
    use crate::prelude::*;
    use bevy::prelude::*;

    #[derive(Resource)]
    struct ExpectedFinalFrame(u32);

    #[derive(Resource, Default)]
    struct Frame(u32);

    fn tick_frame(mut frame: ResMut<Frame>) {
        frame.0 += 1;
        info!("Frame -> {}", frame.0);
    }

    // returns success or failure immediately
    #[derive(Clone)]
    struct CheckTrigger(bool);

    fn on_check_trigger(
        t: Trigger<BehaveTrigger<CheckTrigger>>,
        mut commands: Commands,
        frame: Res<Frame>,
    ) {
        let CheckTrigger(success) = t.inner();
        info!("CheckTrigger @ {} returning {}", frame.0, success);
        if *success {
            commands.trigger(t.ctx().success());
        } else {
            commands.trigger(t.ctx().failure());
        }
    }

    // run once the topmost tree completes.
    fn final_tree_finished_checker(
        _t: Trigger<OnAdd, BehaveFinished>,
        mut exit: EventWriter<AppExit>,
        frame: Res<Frame>,
        expected_final_frame: Res<ExpectedFinalFrame>,
    ) {
        info!("Finished @ {}", frame.0);
        assert_eq!(
            frame.0, expected_final_frame.0,
            "Mismatch on final frame number"
        );
        exit.write(AppExit::Success);
    }

    let mut app = App::new();
    app.init_resource::<Frame>();
    app.insert_resource(ExpectedFinalFrame(expected_final_frame));
    app.add_observer(on_check_trigger);
    app.add_plugins(MinimalPlugins);
    if sync {
        app.add_plugins(BehavePlugin::default().with_synchronous());
    } else {
        app.add_plugins(BehavePlugin::default());
    }
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_systems(FixedPreUpdate, tick_frame.before(BehaveSet));
    app.add_systems(Startup, |mut commands: Commands| {
        let tree = behave! {
            Behave::Sequence => {
                Behave::trigger(CheckTrigger(true)),
                Behave::spawn_named("Nested tree", BehaveTree::new(behave! { Behave::trigger(CheckTrigger(true)), }).with_logging(true)),
            }
        };
        let id = commands.spawn(BehaveTree::new(tree).with_logging(true))
            .observe(final_tree_finished_checker)
            .id();
        info!("spawned tree with id: {}", id);
    });
    app.add_observer(
        |t: Trigger<OnAdd, BehaveFinished>,
         q: Query<(&BehaveFinished, &BehaveCtx)>,
         mut commands: Commands| {
            let Ok((finished, ctx)) = q.get(t.target()) else {
                // if there was no BehaveCtx on this entity, it was the topmost tree, so just return.
                return;
            };
            if finished.0 {
                commands.trigger(ctx.success());
            } else {
                commands.trigger(ctx.failure());
            }
        },
    );
    app.run();
}

/// Test BehaveInterrupt
#[test]
fn test_behave_interrupt() {
    use crate::prelude::*;
    use bevy::prelude::*;

    #[derive(Component, Clone)]
    struct LongRunningTask;

    #[derive(Clone)]
    struct CheckHasEnemyInRange;

    #[derive(Resource, Default)]
    struct TestState {
        enemy_in_range: bool,
        task_interrupted: bool,
    }

    // The interrupt condition checker
    fn check_enemy_in_range(
        trigger: Trigger<BehaveTrigger<CheckHasEnemyInRange>>,
        test_state: Res<TestState>,
        mut commands: Commands,
    ) {
        info!("Checking for enemy in range: {}", test_state.enemy_in_range);
        if test_state.enemy_in_range {
            commands.trigger(trigger.ctx().success());
        } else {
            commands.trigger(trigger.ctx().failure());
        }
    }

    // Observer to track when the main task gets interrupted
    fn on_task_finished(
        _trigger: Trigger<OnAdd, BehaveFinished>,
        mut test_state: ResMut<TestState>,
        mut exit: EventWriter<AppExit>,
    ) {
        test_state.task_interrupted = true;
        info!("Task was interrupted successfully!");
        exit.write(AppExit::Success);
    }

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(BehavePlugin::default());
    app.add_plugins(bevy::log::LogPlugin::default());
    app.init_resource::<TestState>();

    app.add_observer(check_enemy_in_range);
    app.add_observer(on_task_finished);

    app.add_systems(
        Startup,
        |mut commands: Commands, mut test_state: ResMut<TestState>| {
            // Set enemy in range after a few frames
            test_state.enemy_in_range = true;

            let tree = behave! {
                Behave::spawn_named("Long task with interrupt", (
                    LongRunningTask,
                    BehaveInterrupt::new(CheckHasEnemyInRange),
                ))
            };

            commands.spawn(BehaveTree::new(tree).with_logging(true));
        },
    );

    app.run();
}

/// asserts the tree.to_string matches the expected string, accounting for whitespace/indentation
fn assert_tree(s: &str, tree: Tree<Behave>) {
    // strip and tidy any indent spaces in the expected output so we can easily compare
    let leading_spaces = s
        .lines()
        .find(|line| !line.trim().is_empty() && line.starts_with(' '))
        .map(|line| line.len() - line.trim_start().len())
        .unwrap_or(0);
    let mut expected = s
        .lines()
        .map(|line| {
            if line.len() >= leading_spaces {
                &line[leading_spaces..]
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    expected.push('\n');
    assert_eq!(tree.to_string(), expected);
}
