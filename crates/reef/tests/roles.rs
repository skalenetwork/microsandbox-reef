use std::path::{Path, PathBuf};

fn catalog(dir: &str) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(dir),
    )
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
}

#[test]
fn every_catalog_role_parses() {
    let mut checked = 0;
    for path in catalog("roles") {
        let text = std::fs::read_to_string(&path).unwrap();
        reef_core::parse_role(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        checked += 1;
    }
    assert!(checked >= 2, "the roles catalog is missing");
}

#[test]
fn every_catalog_fleet_parses() {
    let mut checked = 0;
    for path in catalog("fleet") {
        let text = std::fs::read_to_string(&path).unwrap();
        reef_core::parse_fleet(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        checked += 1;
    }
    assert!(checked >= 1, "the fleet example is missing");
}
