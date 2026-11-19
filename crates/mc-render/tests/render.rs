//! Baking and meshing against the vanilla resources and the vanilla-saved test world.

use mc_render::bake::BlockModels;
use mc_render::mesh::{Mesher, WorldView};
use mc_render::tint::Biomes;

fn setup() -> (mc_assets::Resources, mc_registry::Registry, BlockModels) {
    let res = mc_assets::Resources::new(mc_assets::default_root());
    let registry = mc_registry::Registry::vanilla().unwrap();
    let models = BlockModels::bake(&res, &registry, &[]).unwrap();
    (res, registry, models)
}

/// A sprite that fails to load falls back to the purple and black checkerboard without an
/// error, so check that no block face ended up with it.
#[test]
fn every_block_face_finds_its_texture() {
    let (_, registry, models) = setup();
    let missing = models.atlas.sprite(mc_assets::MISSING_SPRITE).rect;
    let inside = |uv: [f32; 2]| {
        uv[0] >= missing[0] && uv[0] <= missing[2] && uv[1] >= missing[1] && uv[1] <= missing[3]
    };
    let mut faces = 0;
    let mut bad = Vec::new();
    for (state, model) in models.states.iter().enumerate() {
        for (_, baked) in model.parts.iter().flatten() {
            for quad in &baked.quads {
                faces += 1;
                if quad.uv.iter().all(|&uv| inside(uv)) {
                    bad.push(registry.block_of_state(state as u32).unwrap().name.clone());
                }
            }
        }
    }
    bad.dedup();
    assert!(
        bad.is_empty(),
        "{} blocks draw the missing texture: {:?}",
        bad.len(),
        &bad[..bad.len().min(20)]
    );
    assert!(faces > 100_000, "only {faces} faces baked");
}

/// The stone placed alone at y=250 in the test world: six faces, nothing hidden, all lit by
/// the open sky even though the save leaves those sections' sky light out. (Below the stone
/// the sky light is 14, as in vanilla, so the bottom face's corners average a little under 15.)
#[test]
fn a_lone_block_in_the_sky_gets_six_sky_lit_faces() {
    let (res, registry, models) = setup();
    let mut world = mc_world::World::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/worlds/test-world"
    ));
    let view = WorldView::load(&mut world, &registry, -24, 1, 1).unwrap();
    let biomes = Biomes::load(&res, view.biome_names()).unwrap();
    let mesh = Mesher::new(&models, &biomes, &registry).mesh_world(&view);

    let in_stone = |v: &mc_render::mesh::Vertex| {
        let [x, y, z] = v.pos;
        (-24.0..=-23.0).contains(&x) && (250.0..=251.0).contains(&y) && (1.0..=2.0).contains(&z)
    };
    let all: Vec<_> = mesh
        .opaque
        .iter()
        .chain(&mesh.cutout)
        .chain(&mesh.translucent)
        .collect();
    let vertices: Vec<_> = all.iter().filter(|v| in_stone(v)).collect();
    assert_eq!(vertices.len(), 6 * 4, "the stone's faces");
    assert!(
        vertices
            .iter()
            .all(|v| v.light[0] >= 14 * 16 && v.light[1] == 0),
        "sky light 14 to 15, block light 0"
    );
    let top: Vec<_> = vertices.iter().filter(|v| v.pos[1] == 251.0).collect();
    assert!(
        top.iter().all(|v| v.light[0] == 15 * 16),
        "the top face is under open sky"
    );
}

/// Every block draws something except invisible ones and fluids, which the fluid mesher
/// draws. A block that silently loses its model shows up here.
#[test]
fn only_known_blocks_draw_nothing() {
    let (_, registry, models) = setup();
    let invisible = [
        "air",
        "cave_air",
        "void_air",
        "barrier",
        "light",
        "structure_void",
        "moving_piston",
    ];
    // Drawn by the fluid mesher.
    let fluids = ["water", "lava", "bubble_column"];
    let mut empty = Vec::new();
    for block in registry.blocks() {
        let name = block.name.strip_prefix("minecraft:").unwrap();
        let state = &models.states[block.default_state as usize];
        let quads: usize = state
            .parts
            .iter()
            .flatten()
            .map(|(_, m)| m.quads.len())
            .sum();
        let known = invisible.contains(&name) || fluids.contains(&name);
        if quads == 0 && !known {
            empty.push(name.to_owned());
        }
        // Drawn in code by vanilla's block entity renderers.
        let entity_drawn = [
            "chest",
            "banner",
            "shulker_box",
            "skull",
            "_head",
            "decorated_pot",
            "conduit",
            "copper_golem_statue",
            "end_portal",
            "end_gateway",
        ];
        if entity_drawn.iter().any(|e| name.ends_with(e)) {
            assert!(quads > 0, "{name} draws nothing");
        }
    }
    assert!(empty.is_empty(), "blocks that draw nothing: {empty:?}");
}

/// Block entity data placed in the test world changes what's drawn: the white banner's
/// creeper pattern, and the pot's skull sherd on its front.
#[test]
fn block_entity_data_draws_patterns_and_sherds() {
    let (res, registry, models) = setup();
    let mut world = mc_world::World::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/worlds/test-world"
    ));
    let view = WorldView::load(&mut world, &registry, 24, -12, 1).unwrap();
    let biomes = Biomes::load(&res, view.biome_names()).unwrap();
    let mesh = Mesher::new(&models, &biomes, &registry).mesh_world(&view);
    let uses = |sprite: &str, block: [f32; 3]| {
        let r = models.atlas.sprite(sprite).rect;
        mesh.opaque
            .iter()
            .chain(&mesh.cutout)
            .chain(&mesh.translucent)
            .filter(|v| (0..3).all(|i| (v.pos[i] - block[i] - 0.5).abs() < 1.5))
            .any(|v| (r[0]..=r[2]).contains(&v.uv[0]) && (r[1]..=r[3]).contains(&v.uv[1]))
    };
    assert!(uses("entity/banner/creeper", [24.0, 250.0, -8.0]));
    assert!(uses(
        "entity/decorated_pot/skull_pottery_pattern",
        [24.0, 250.0, -14.0]
    ));
    // A plain pot elsewhere doesn't pick up a sherd.
    assert!(!uses(
        "entity/decorated_pot/skull_pottery_pattern",
        [24.0, 250.0, -2.0]
    ));
}

/// End portals and gateways go to the portal shader, with vanilla's 15 and 16 star layers,
/// and nowhere else.
#[test]
fn portals_draw_with_the_portal_shader() {
    let (res, registry, models) = setup();
    let mut world = mc_world::World::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/worlds/test-world"
    ));
    let view = WorldView::load(&mut world, &registry, -85, 1, 1).unwrap();
    let biomes = Biomes::load(&res, view.biome_names()).unwrap();
    let mesh = Mesher::new(&models, &biomes, &registry).mesh_world(&view);
    // Within the block at (x, 250, 1).
    let at = |v: &mc_render::mesh::Vertex, x: f32| {
        [x, 250.0, 1.0]
            .iter()
            .zip(v.pos)
            .all(|(min, p)| (p - min - 0.5).abs() <= 0.5)
    };
    // The end portal at x = -84 has a top and a bottom face; the gateway at -87 all six.
    let portal: Vec<_> = mesh.portal.iter().filter(|v| at(v, -84.0)).collect();
    let gateway: Vec<_> = mesh.portal.iter().filter(|v| at(v, -87.0)).collect();
    assert_eq!(portal.len(), 2 * 4);
    assert_eq!(gateway.len(), 6 * 4);
    assert!(portal.iter().all(|v| v.light[2] == 15));
    assert!(gateway.iter().all(|v| v.light[2] == 16));
    let elsewhere = mesh
        .opaque
        .iter()
        .chain(&mesh.cutout)
        .chain(&mesh.translucent)
        .any(|v| at(v, -84.0) || at(v, -87.0));
    assert!(!elsewhere, "portal faces in a terrain layer");
}
