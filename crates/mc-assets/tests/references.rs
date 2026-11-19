//! Every block's blockstate → model → texture chain resolves against `resources/`.

use mc_assets::{MISSING_SPRITE, Resources, default_root};

#[test]
fn every_block_model_and_texture_resolves() {
    let res = Resources::new(default_root());
    let blocks = res.block_names().unwrap();
    assert!(
        blocks.len() > 1000,
        "only {} blockstates; run `cargo xtask fetch-assets`",
        blocks.len()
    );

    let mut misses = Vec::new();
    let mut faces = 0;
    for block in &blocks {
        let state = match res.blockstate(block) {
            Ok(state) => state,
            Err(e) => {
                misses.push(format!("{block}: {e:#}"));
                continue;
            }
        };
        for variant in state.all_variants() {
            let model = match res.model(&variant.model) {
                Ok(model) => model,
                Err(e) => {
                    misses.push(format!("{block}: {e:#}"));
                    continue;
                }
            };
            let face_textures = model
                .elements
                .iter()
                .flatten()
                .flat_map(|e| e.faces.values())
                .map(|f| f.texture.as_str());
            for reference in face_textures.chain(
                model
                    .textures
                    .contains_key("particle")
                    .then_some("particle"),
            ) {
                faces += 1;
                match model.sprite(reference) {
                    Some(MISSING_SPRITE) => {}
                    Some(sprite) if res.asset_path("textures", sprite, "png").is_file() => {}
                    Some(sprite) => misses.push(format!(
                        "{block}: {} uses missing texture {sprite}",
                        variant.model
                    )),
                    None => misses.push(format!(
                        "{block}: {} leaves {reference} unresolved",
                        variant.model
                    )),
                }
            }
        }
    }
    misses.sort();
    misses.dedup();
    assert!(
        misses.is_empty(),
        "{} misses, first ones:\n{}",
        misses.len(),
        misses[..misses.len().min(20)].join("\n")
    );
    assert!(faces > 10_000, "only {faces} texture references checked");
}
