//! The registry against vanilla 26.3's own reports and blockstate files.

use mc_registry::{Registry, default_reports_dir};
use serde_json::Value;

/// Every state id we compute from property values, and every property value we decode from
/// a state id, matches the id and values the data generator listed for that state.
#[test]
fn state_ids_match_the_data_generator() {
    let registry = Registry::vanilla().unwrap();
    let report: Value =
        serde_json::from_slice(&std::fs::read(default_reports_dir().join("blocks.json")).unwrap())
            .unwrap();
    let mut checked = 0;
    for (name, block_report) in report.as_object().unwrap() {
        let block = registry.block(name).unwrap();
        for state in block_report["states"].as_array().unwrap() {
            let id = state["id"].as_u64().unwrap() as u32;
            let props = &state["properties"];
            assert_eq!(block.state(|p| props[p].as_str()), id, "{name} {props}");
            for (property, value) in block.property_values(id) {
                assert_eq!(props[property].as_str(), Some(value), "{name} state {id}");
            }
            assert_eq!(registry.block_of_state(id).unwrap().name, *name);
            assert_eq!(
                state["default"].as_bool().unwrap_or(false),
                block.default_state == id,
                "{name} state {id}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, registry.state_count());
    assert!(checked > 30_000, "only {checked} states");
}

/// Properties a save leaves out, or gives values we don't know, keep their defaults.
#[test]
fn missing_and_unknown_properties_keep_defaults() {
    let registry = Registry::vanilla().unwrap();
    let stairs = registry.block("minecraft:oak_stairs").unwrap();
    let state = stairs.state(|p| match p {
        "facing" => Some("west"),
        "half" => Some("sideways"), // not a value stairs have
        _ => None,
    });
    let values = stairs.property_values(state);
    let default = stairs.property_values(stairs.default_state);
    for ((name, value), (_, default_value)) in values.iter().zip(&default) {
        let expected = if *name == "facing" {
            "west"
        } else {
            default_value
        };
        assert_eq!(value, &expected, "{name}");
    }
}

/// Every state of every block picks its models from the block's blockstate file: exactly
/// one entry of a `variants` file, at least one case of a `multipart` file.
#[test]
fn every_block_state_selects_a_model() {
    let registry = Registry::vanilla().unwrap();
    let assets = mc_assets::Resources::new(mc_assets::default_root());
    let mut problems = Vec::new();
    for block in registry.blocks() {
        let file = assets.blockstate(&block.name).unwrap();
        let multipart = matches!(file, mc_assets::BlockState::Multipart(_));
        for state in block.first_state..block.first_state + block.state_count() {
            let values = block.property_values(state);
            let value = |p: &str| values.iter().find(|(n, _)| *n == p).map(|(_, v)| *v);
            let selected = file.select(&value);
            // A wall with no sides and no post selects no part, so vanilla draws nothing. The
            // game never places one (no sides means a post); commands and the debug world can.
            let bare_wall = block.name.ends_with("_wall")
                && value("up") == Some("false")
                && ["north", "east", "south", "west"]
                    .iter()
                    .all(|side| value(side) == Some("none"));
            if bare_wall {
                assert!(selected.is_empty(), "{} {values:?}", block.name);
                continue;
            }
            if selected.len() != 1 && !(multipart && !selected.is_empty()) {
                problems.push(format!(
                    "{} {values:?}: {} matches",
                    block.name,
                    selected.len()
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} states, first ones:\n{}",
        problems.len(),
        problems[..problems.len().min(20)].join("\n")
    );
}
