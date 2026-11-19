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

## Crates

- `mc-assets`: blockstates, block models (parents merged), choosing a state's models
- `mc-nbt`: NBT read/write, file and network forms
- `mc-registry`: blocks, block states and items with vanilla's ids

## Checks

```sh
cargo test
```

These run against the vanilla files in `resources/`: every blockstate → model → texture
reference resolves, all 1,511 vanilla structure `.nbt` files read and write back byte for
byte, all 35,723 block state ids match vanilla's, and every block state selects its models.
