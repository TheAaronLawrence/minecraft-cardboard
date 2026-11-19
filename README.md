# minecraft-cardboard 0.0.1

A Rust rebuild of Minecraft: Java Edition. Our code is our own; the game data under
`resources/` is Mojang's, copied from vanilla **26.3**, the version whose data, protocol and
save format we match.

## Resources

`resources/` holds the vanilla `assets/` and `data/` trees (textures, models, blockstates,
shaders, sounds, languages, worldgen, recipes, loot tables, tags). `resources/source.json`
records exactly which vanilla files they came from. To refresh them:

```sh
cargo xtask fetch-assets        # or: cargo xtask fetch-assets <vanilla version>
```

It downloads from Mojang's public servers, checks every file's SHA-1, and copies only
assets and data, never game code. Keep this repository private: Mojang's EULA does not allow
redistributing these files.

`resources/reports/` holds what vanilla defines in code rather than data: every block's
properties and numeric state ids (`blocks.json`) and every registry's protocol ids
(`registries.json`). They come from vanilla's own data generator, which needs Java 25:

```sh
cargo xtask gen-reports         # set JAVA=/path/to/java if it isn't on PATH
```

`fixtures/worlds/test-world` is a world vanilla itself generated and saved, for tests and
for trying the renderer. `cargo xtask gen-world` recreates it: it runs the vanilla 26.3
server offline (which accepts Mojang's EULA), generates 12×12 chunks from a fixed seed, and
places the blocks listed in `fixtures/worlds/test-world.blocks`.

## Running

```sh
cargo run --release -p mc-client -- --pos 20.5,78,-30.5 --look 210,15
```

Opens the test world (or any save with `--world DIR`) in a window: click to capture the
mouse, WASD to move, Space and Shift for up and down, Ctrl to go faster, Esc to let go.
Add `--screenshot out.png` to render one frame to a file instead; it works without a GPU
through Mesa's lavapipe Vulkan driver.

## Crates

- `mc-assets`: blockstates, block models (parents merged), choosing a state's models
- `mc-nbt`: NBT read/write, file and network forms
- `mc-registry`: blocks, block states and items with vanilla's ids
- `mc-world`: saves: region files, chunks, block states, biomes, light
- `mc-render`: texture atlas, model baking, chunk meshing with smooth lighting, wgpu renderer
- `mc-client`: the `minecraft-cardboard` binary

## Checks

```sh
cargo test
```

These run against the vanilla files in `resources/`: every blockstate → model → texture
reference resolves, all 1,511 vanilla structure `.nbt` files read and write back byte for
byte, all 35,723 block state ids match vanilla's, every block state selects its models,
the test world reads back exactly the blocks vanilla was told to place, and every block face
finds its texture.
