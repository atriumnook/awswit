//! Fuzzy matching tests

use strsim::jaro_winkler;

#[test]
fn test_fuzzy_exact_match() {
    let similarity = jaro_winkler("dev-admin", "dev-admin");
    assert!(similarity > 0.99);
}

#[test]
fn test_fuzzy_prefix_match() {
    let similarity = jaro_winkler("dev", "dev-admin");
    assert!(similarity > 0.7);
}

#[test]
fn test_fuzzy_typo() {
    let similarity = jaro_winkler("dev-admni", "dev-admin");
    assert!(similarity > 0.9);
}

#[test]
fn test_fuzzy_completely_different() {
    let similarity = jaro_winkler("xyz", "dev-admin");
    assert!(similarity < 0.5);
}
