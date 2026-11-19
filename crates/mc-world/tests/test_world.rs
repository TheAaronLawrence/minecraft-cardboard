//! Reads `fixtures/worlds/test-world`, which the vanilla server generated and saved
//! (`cargo xtask gen-world`), and checks it against what vanilla was told to put there.

use mc_registry::Registry;
use mc_world::World;
use std::path::Path;

fn fixtures() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/worlds"
    ))
}

/// The 12×12 chunks around the origin, all fully generated.
const CHUNKS: std::ops::Range<i32> = -6..6;

#[test]
fn placed_blocks_read_back_as_placed() {
    let registry = Registry::vanilla().unwrap();
    let mut world = World::open(fixtures().join("test-world"));
    let placements = std::fs::read_to_string(fixtures().join("test-world.blocks")).unwrap();
    let mut checked = 0;
    for line in placements
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut parts = line.splitn(4, ' ');
        let [x, y, z]: [i32; 3] = std::array::from_fn(|_| parts.next().unwrap().parse().unwrap());
        // Block entity data, such as `{patterns:[...]}`, follows the state.
        let state = parts.next().unwrap();
        let state = state.split_once('{').map_or(state, |(state, _)| state);
        let expected = parse_state(&registry, state);
        let chunk = world
            .chunk(&registry, x.div_euclid(16), z.div_euclid(16))
            .unwrap()
            .unwrap();
        let actual = chunk.block(x, y, z).unwrap();
        let describe = |id| {
            let block = registry.block_of_state(id).unwrap();
            format!("{} {:?}", block.name, block.property_values(id))
        };
        assert_eq!(describe(actual), describe(expected), "at {x} {y} {z}");
        // The block below sits in the same air, so the reader isn't just returning one value.
        assert_eq!(
            registry
                .block_of_state(chunk.block(x, y - 1, z).unwrap())
                .unwrap()
                .name,
            "minecraft:air"
        );
        checked += 1;
    }
    assert!(checked >= 17, "only {checked} placements");
}

#[test]
fn every_chunk_is_generated_with_a_bedrock_floor() {
    let registry = Registry::vanilla().unwrap();
    let bedrock = registry.block("minecraft:bedrock").unwrap().default_state;
    let mut world = World::open(fixtures().join("test-world"));
    for cx in CHUNKS {
        for cz in CHUNKS {
            let chunk = world
                .chunk(&registry, cx, cz)
                .unwrap()
                .unwrap_or_else(|| panic!("chunk {cx},{cz} missing"));
            assert_eq!((chunk.x, chunk.z), (cx, cz));
            assert_eq!(chunk.status, "minecraft:full", "chunk {cx},{cz}");
            // The overworld's lowest layer is always bedrock, so every column checks the
            // bottom section's unpacking at every x and z.
            for x in cx * 16..cx * 16 + 16 {
                for z in cz * 16..cz * 16 + 16 {
                    assert_eq!(chunk.block(x, -64, z), Some(bedrock), "at {x} -64 {z}");
                }
            }
            // Something other than air and bedrock, so terrain was decoded too.
            let surface = (cx * 16..cx * 16 + 16).any(|x| {
                (0..64).any(|y| {
                    chunk.block(x, y, cz * 16).is_some_and(|s| {
                        s != bedrock && registry.block_of_state(s).unwrap().name != "minecraft:air"
                    })
                })
            });
            assert!(surface, "chunk {cx},{cz} has no terrain below y=64");
        }
    }
    assert!(
        world.chunk(&registry, 100, 100).unwrap().is_none(),
        "a chunk that was never generated"
    );
}

/// `minecraft:oak_stairs[facing=west,half=top]` as a state id; unlisted properties default.
fn parse_state(registry: &Registry, text: &str) -> u32 {
    let (name, props) = text
        .split_once('[')
        .map_or((text, ""), |(n, rest)| (n, rest.trim_end_matches(']')));
    let pairs: Vec<(&str, &str)> = props
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').unwrap())
        .collect();
    let block = registry
        .block(name)
        .unwrap_or_else(|| panic!("unknown block {name}"));
    for (property, _) in &pairs {
        assert!(
            block.properties.iter().any(|p| p.name == *property),
            "{name} has no property {property}"
        );
    }
    block.state(|p| pairs.iter().find(|(k, _)| *k == p).map(|(_, v)| *v))
}
