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
