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

## Checks

```sh
cargo test    # includes: every blockstate → model → texture reference resolves
```
