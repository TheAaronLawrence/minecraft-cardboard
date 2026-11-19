//! Baking and meshing against the vanilla resources and the vanilla-saved test world.

use mc_render::bake::BlockModels;
use mc_render::mesh::{Mesher, WorldView};
use mc_render::tint::Biomes;

fn setup() -> (mc_assets::Resources, mc_registry::Registry, BlockModels) {
    let res = mc_assets::Resources::new(mc_assets::default_root());
    let registry = mc_registry::Registry::vanilla().unwrap();
    let models = BlockModels::bake(&res, &registry).unwrap();
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
