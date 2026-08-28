use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn fixtures_manifest_sha256_matches() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = fs::read_to_string(root.join("fixtures/vendor/SHA256SUMS"))
        .unwrap_or_else(|e| panic!("reading manifest: {e}"));
    let mut checked = 0;
    for line in manifest.lines() {
        let (want, relative) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("bad manifest row: {line}"));
        let data =
            fs::read(root.join(relative)).unwrap_or_else(|e| panic!("reading {relative}: {e}"));
        assert_eq!(hex(&Sha256::digest(data)), want, "{relative}");
        checked += 1;
    }
    assert_eq!(checked, 20);
    assert_eq!(
        fs::read_dir(root.join("fixtures/vendor/examples"))
            .unwrap_or_else(|e| panic!("reading examples: {e}"))
            .count(),
        17
    );
}
