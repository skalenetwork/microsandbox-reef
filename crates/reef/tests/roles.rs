use std::path::Path;

#[test]
fn every_catalog_role_parses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roles");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            let text = std::fs::read_to_string(&path).unwrap();
            reef_core::parse_role(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            checked += 1;
        }
    }
    assert!(checked >= 2, "the roles catalog is missing");
}
