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
properties and numeric state ids (`blocks.json`), every registry's protocol ids
(`registries.json`), and every packet's id (`packets.json`); `source.json` records the
protocol version. `physics.json` holds every state's collision shape and the blocks'
friction, speed and jump factors, dumped by `xtask/java/DumpPhysics.java` (this step needs
a JDK, not just a JRE). They come from vanilla's own data generator, which needs Java 25:

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

```sh
cargo run --release -p mc-client -- --server localhost:25565 --name cardboard
```

Joins a vanilla 26.3 server instead, and draws the chunks it sends. On a server you walk
with vanilla's movement and collision: WASD, Space to jump, Shift to sneak, Ctrl to sprint,
and double-tap Space to fly where the server allows it. Only offline-mode servers
(`online-mode=false`) work so far. Swimming isn't simulated yet. In survival the HUD shows
hearts, food and experience, and dying opens the death screen.

```sh
cargo build --release -p mc-client && cargo run --release -p mc-launcher
```

Opens the launcher, styled after Mojang's: set a player name and press Play. It starts the
`minecraft-cardboard` beside it, remembers the name in `launcher.json` beside it, and comes
back when the game closes. Started without `--world` or `--server`, the game opens on its
title screen: Singleplayer opens the test world, and Multiplayer asks for a server's
address, remembered as vanilla does in `options.txt`. Leaving a server, or losing the
connection, comes back to the menus.

## Crates

- `mc-assets`: blockstates, block models (parents merged), choosing a state's models
- `mc-nbt`: NBT read/write, file and network forms
- `mc-registry`: blocks, block states and items with vanilla's ids
- `mc-world`: saves: region files, chunks, block states, biomes, light
- `mc-protocol`: the network protocol: framing, compression, login, configuration, and the
  play packets for joining and receiving chunks
- `mc-physics`: the player's movement and collision, as vanilla's
- `mc-render`: texture atlas, model baking, chunk meshing with smooth lighting, wgpu renderer
- `mc-client`: the `minecraft-cardboard` binary
- `mc-launcher`: the `minecraft-cardboard-launcher` binary, which starts the game

## Checks

```sh
cargo test
```

These run against the vanilla files in `resources/`: every blockstate → model → texture
reference resolves, all 1,511 vanilla structure `.nbt` files read and write back byte for
byte, all 35,723 block state ids match vanilla's, every block state selects its models,
the test world reads back exactly the blocks vanilla was told to place, and every block face
finds its texture.
