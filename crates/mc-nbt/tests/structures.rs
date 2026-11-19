//! Every vanilla structure file reads and writes back byte for byte.

use std::io::Read;

#[test]
fn vanilla_structure_files_round_trip() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../resources/data/minecraft/structure");
    let mut files = Vec::new();
    collect(&dir, &mut files);
    assert!(
        files.len() > 1000,
        "only {} structure files; run `cargo xtask fetch-assets`",
        files.len()
    );

    for path in &files {
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
            .read_to_end(&mut raw)
            .unwrap();
        let (name, root) =
            mc_nbt::from_bytes(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            root.contains_key("blocks") && root.contains_key("palette")
                || root.contains_key("palettes"),
            "{}",
            path.display()
        );
        assert!(
            mc_nbt::to_bytes(&name, &root) == raw,
            "{} changed when written back",
            path.display()
        );
    }
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "nbt") {
            out.push(path);
        }
    }
}
