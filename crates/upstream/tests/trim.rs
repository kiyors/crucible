//! `update rust` rewrites `data/rust/*.json` on every run, so the trimming rules
//! are compared against a checked-in expectation rather than a live manifest.

use upstream::manifest::Manifest;

#[test]
fn trims_a_channel_manifest_exactly() {
    let manifest: Manifest = toml::from_str(include_str!("fixtures/channel.toml")).unwrap();

    let json = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest.trim()).unwrap()
    );

    assert_eq!(json, include_str!("fixtures/channel.trimmed.json"));
}
