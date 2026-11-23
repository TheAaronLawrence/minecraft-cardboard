use super::*;
use std::collections::HashMap;

/// Stone up to y 63 wherever `floor` says, air above, plus placed blocks.
struct Test {
    blocks: HashMap<[i32; 3], u32>,
    floor: fn(i32, i32) -> bool,
    stone: u32,
    air: u32,
}

impl World for Test {
    fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        match self.blocks.get(&[x, y, z]) {
            Some(&s) => s,
            None if y < 64 && (self.floor)(x, z) => self.stone,
            None => self.air,
        }
    }
    fn loaded(&self, _: i32, _: i32) -> bool {
        true
    }
}

fn setup(floor: fn(i32, i32) -> bool) -> (Registry, BlockPhysics, Test) {
    let registry = Registry::vanilla().unwrap();
    let physics = BlockPhysics::vanilla(&registry).unwrap();
    let state = |name: &str| registry.block(name).unwrap().default_state;
    let world = Test {
        blocks: HashMap::new(),
        floor,
        stone: state("minecraft:stone"),
        air: state("minecraft:air"),
    };
    (registry, physics, world)
}

fn run(p: &mut Player, input: Input, ticks: usize, w: &Test, b: &BlockPhysics) {
    for _ in 0..ticks {
        p.tick(input, w, b);
    }
}

const FORWARD: Input = Input {
    forward: true,
    backward: false,
    left: false,
    right: false,
    jump: false,
    sneak: false,
    sprint: false,
};

/// Vanilla's published speeds: walking 4.317 m/s and sprinting 5.612 m/s, i.e. 0.21585 and
/// 0.2806 blocks a tick, facing south (+z) at yaw 0.
#[test]
fn walk_and_sprint_speeds() {
    let (_, physics, world) = setup(|_, _| true);
    for (input, expected) in [
        (FORWARD, 4.317),
        (
            Input {
                sprint: true,
                ..FORWARD
            },
            5.612,
        ),
    ] {
        let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
        run(&mut p, input, 100, &world, &physics);
        let before = p.pos;
        p.tick(input, &world, &physics);
        let step = p.pos - before;
        assert!(
            (step.z * 20.0 - expected).abs() < 0.002,
            "{input:?}: {} m/s",
            step.z * 20.0
        );
        assert!(step.x.abs() < 1e-9 && step.y == 0.0 && p.on_ground);
    }
}

/// A standing jump peaks 1.2522 blocks up and lands back on the ground.
#[test]
fn jump_height() {
    let (_, physics, world) = setup(|_, _| true);
    let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
    run(&mut p, Input::default(), 5, &world, &physics);
    let jump = Input {
        jump: true,
        ..Input::default()
    };
    p.tick(jump, &world, &physics);
    let mut top: f64 = 0.0;
    for _ in 0..30 {
        p.tick(Input::default(), &world, &physics);
        top = top.max(p.pos.y - 64.0);
    }
    assert!((top - 1.2522).abs() < 1e-3, "peaked {top}");
    assert!(p.on_ground && p.pos.y == 64.0);
}

/// Walking steps up onto a slab, but a two-high wall stops the player against it.
#[test]
fn steps_up_slabs_and_stops_at_walls() {
    let (registry, physics, mut world) = setup(|_, _| true);
    let slab = registry.block("minecraft:smooth_stone_slab").unwrap();
    let bottom = slab.state(|p| (p == "type").then_some("bottom"));
    world.blocks.insert([0, 64, 2], bottom);
    let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
    run(&mut p, FORWARD, 40, &world, &physics);
    assert!(
        p.pos.z > 3.0 && p.pos.y == 64.0,
        "didn't cross the slab: {}",
        p.pos
    );

    world.blocks.insert([0, 64, 2], world.stone);
    world.blocks.insert([0, 65, 2], world.stone);
    let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
    run(&mut p, FORWARD, 40, &world, &physics);
    assert!((p.pos.z - 1.7).abs() < 1e-6 && p.pos.y == 64.0, "{}", p.pos);
    assert!(p.horizontal_collision);
}

/// Sneaking stops at the edge of a ledge; walking the same way falls off it.
#[test]
fn sneaking_stays_on_ledges() {
    let (_, physics, world) = setup(|_, z| z <= 1);
    let sneak = Input {
        sneak: true,
        ..FORWARD
    };
    let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
    run(&mut p, sneak, 100, &world, &physics);
    assert!(
        p.pos.y == 64.0 && p.pos.z < 2.3 && p.pos.z > 2.2,
        "{}",
        p.pos
    );

    let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
    run(&mut p, FORWARD, 40, &world, &physics);
    assert!(p.pos.y < 63.0, "{}", p.pos);
}

/// Looking straight down hits the top of the floor; looking level at short grass hits its
/// outline (2 px in from the block edge), not the block's full cube, and a ray over the
/// grass's 13 px top passes it by.
#[test]
fn picks_block_outlines() {
    let (registry, physics, mut world) = setup(|_, _| true);
    let grass = registry
        .block("minecraft:short_grass")
        .unwrap()
        .default_state;
    world.blocks.insert([0, 64, 3], grass);

    let eye = DVec3::new(0.5, 65.62, 0.5);
    let down = view_vector(0.0, 90.0);
    let hit = pick(&world, &physics, eye, eye + down * 4.5).unwrap();
    assert_eq!(
        (hit.pos, hit.face, hit.inside),
        (glam::IVec3::new(0, 63, 0), 1, false)
    );
    assert!((hit.location.y - 64.0).abs() < 1e-9);

    // Yaw 0 looks south, toward +z.
    let low = DVec3::new(0.5, 64.5, 0.5);
    let hit = pick(&world, &physics, low, low + view_vector(0.0, 0.0) * 4.5).unwrap();
    assert_eq!((hit.pos, hit.face), (glam::IVec3::new(0, 64, 3), 2));
    assert!((hit.location.z - 3.125).abs() < 1e-6, "{hit:?}");

    let high = DVec3::new(0.5, 64.9, 0.5);
    assert_eq!(
        pick(&world, &physics, high, high + view_vector(0.0, 0.0) * 4.5),
        None
    );
}

/// Vanilla's bare-handed break times: stone 7.5 s (150 ticks), dirt 0.75 s, five times
/// slower in the air, and bedrock never.
#[test]
fn destroy_progress_matches_vanilla() {
    let (registry, physics, _) = setup(|_, _| true);
    let state = |name: &str| registry.block(name).unwrap().default_state;
    let ticks = |name: &str, on_ground: bool| {
        (1.0 / physics.destroy_progress(state(name), on_ground)).ceil()
    };
    assert_eq!(ticks("minecraft:stone", true), 150.0);
    assert_eq!(ticks("minecraft:dirt", true), 15.0);
    assert_eq!(ticks("minecraft:dirt", false), 75.0);
    assert_eq!(
        physics.destroy_progress(state("minecraft:bedrock"), true),
        0.0
    );
    assert!(physics.destroy_progress(state("minecraft:short_grass"), true) >= 1.0);
}

/// In still water, from `travelInWater`'s 0.8 drag (0.9 sprinting), 0.02 strokes and 1/16 of
/// gravity: sinking settles at 0.025 a tick (0.5 m/s), walking along the bottom at
/// 0.0196 / 0.2 a tick (1.96 m/s), and sprinting under water swims at 0.0196 / 0.1 a tick
/// (3.92 m/s), lying down with the eye 0.4 up.
#[test]
fn swims_in_still_water() {
    let (registry, physics, mut world) = setup(|_, _| true);
    let water = registry.block("minecraft:water").unwrap().default_state;
    for x in -3..=3 {
        for y in 64..=71 {
            for z in -3..=40 {
                world.blocks.insert([x, y, z], water);
            }
        }
    }
    let step = |p: &mut Player, input: Input| {
        let before = p.pos;
        p.tick(input, &world, &physics);
        p.pos - before
    };

    let mut p = Player::new(DVec3::new(0.5, 69.0, 0.5), 0.0, 0.0);
    run(&mut p, Input::default(), 60, &world, &physics);
    let sink = step(&mut p, Input::default());
    assert!((sink.y + 0.025).abs() < 1e-6, "sank {}", sink.y);
    assert!(p.in_water && p.under_water && !p.swimming);

    let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
    run(&mut p, FORWARD, 60, &world, &physics);
    let walk = step(&mut p, FORWARD);
    assert!(
        (walk.z * 20.0 - 1.96).abs() < 1e-3,
        "walked {} m/s",
        walk.z * 20.0
    );
    assert!(p.on_ground && p.pose == Pose::Standing);

    let sprint = Input {
        sprint: true,
        ..FORWARD
    };
    let mut p = Player::new(DVec3::new(0.5, 66.0, 0.5), 0.0, 0.0);
    run(&mut p, sprint, 120, &world, &physics);
    let swim = step(&mut p, sprint);
    assert!(
        (swim.z * 20.0 - 3.92).abs() < 1e-3,
        "swam {} m/s",
        swim.z * 20.0
    );
    assert!(p.swimming && p.pose == Pose::Swimming && p.eye_height() == 0.4);
}

/// `Entity.applyMovementEmissionAndPlaySound`: a step for each 1/0.6 blocks walked, from the
/// block underfoot; a snow layer on it sounds over it with the stone muffled; sneaking is
/// silent; and a fall that hurts thuds, with the block landed on.
#[test]
fn movement_sounds() {
    let (registry, physics, mut world) = setup(|_, _| true);
    let play = |event: &str, volume: f32, pitch: f32| MoveSound::Play {
        event: event.into(),
        volume,
        pitch,
    };
    let walk = |world: &Test, input: Input| {
        let mut p = Player::new(DVec3::new(0.5, 64.0, 0.5), 0.0, 0.0);
        run(&mut p, input, 100, world, &physics);
        let walked = p.pos.z - 0.5;
        (p.sounds, walked)
    };
    let (sounds, walked) = walk(&world, FORWARD);
    let steps = (walked * 0.6).floor() as usize;
    assert!(
        (steps - 1..=steps).contains(&sounds.len()),
        "{} steps over {walked} blocks",
        sounds.len()
    );
    assert!(
        sounds
            .iter()
            .all(|s| *s == play("minecraft:block.stone.step", 0.15, 1.0))
    );

    let (sounds, _) = walk(
        &world,
        Input {
            sneak: true,
            ..FORWARD
        },
    );
    assert!(sounds.is_empty(), "{sounds:?}");

    let snow = registry.block("minecraft:snow").unwrap().default_state;
    for z in 0..30 {
        world.blocks.insert([0, 64, z], snow);
    }
    let mut p = Player::new(DVec3::new(0.5, 64.125, 0.5), 0.0, 0.0);
    run(&mut p, FORWARD, 40, &world, &physics);
    assert_eq!(
        p.sounds[..2],
        [
            play("minecraft:block.snow.step", 0.15, 1.0),
            play("minecraft:block.stone.step", 0.05, 0.8)
        ]
    );

    world.blocks.clear();
    let still = Input::default();
    let mut p = Player::new(DVec3::new(0.5, 74.0, 0.5), 0.0, 0.0);
    run(&mut p, still, 60, &world, &physics);
    assert_eq!(
        p.sounds,
        [
            play("minecraft:entity.player.big_fall", 1.0, 1.0),
            play("minecraft:block.stone.fall", 0.5, 0.75)
        ]
    );
    // From 4 blocks it hurts by 1: a small fall. From 3 it doesn't hurt, and is silent.
    for (height, sounds) in [(4.0, 2), (3.0, 0)] {
        let mut p = Player::new(DVec3::new(0.5, 64.0 + height, 0.5), 0.0, 0.0);
        run(&mut p, still, 40, &world, &physics);
        assert_eq!(p.sounds.len(), sounds, "from {height}: {:?}", p.sounds);
    }
}
