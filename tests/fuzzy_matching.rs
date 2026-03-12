//! Fuzzy matching tests using the real awswit library

use awswit::profile::Profile;
use awswit::utils::fuzzy::find_closest_profile;
use std::collections::HashMap;

fn profiles(names: &[&str]) -> HashMap<String, Profile> {
    names
        .iter()
        .map(|n| (n.to_string(), Profile::default()))
        .collect()
}

#[test]
fn test_fuzzy_exact_match() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(
        find_closest_profile("dev-admin", &p),
        Some("dev-admin".to_string())
    );
}

#[test]
fn test_fuzzy_prefix_match() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(
        find_closest_profile("stag", &p),
        Some("staging".to_string())
    );
}

#[test]
fn test_fuzzy_typo() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(
        find_closest_profile("stagin", &p),
        Some("staging".to_string())
    );
}

#[test]
fn test_fuzzy_completely_different() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(find_closest_profile("zzzzzzzzzzzzz", &p), None);
}

#[test]
fn test_fuzzy_ambiguous_prefix_returns_none() {
    let p = profiles(&["dev-admin", "dev-readonly"]);
    // "dev" is ambiguous prefix - should return None
    assert_eq!(find_closest_profile("dev", &p), None);
}

#[test]
fn test_fuzzy_equal_levenshtein_returns_none() {
    let p = profiles(&["cat", "bat"]);
    // "hat" has distance 1 from both "cat" and "bat" — tie
    assert_eq!(find_closest_profile("hat", &p), None);
}

#[test]
fn test_fuzzy_lcs_tiebreak_returns_none() {
    // Two profiles where LCS with input is the same length, and no prefix match
    // "axbxc" has LCS 3 with both "aZbZc-one" and "aYbYc-two" (a, b, c)
    // Neither is a prefix of "axbxc"
    let p = profiles(&["aZbZc-one", "aYbYc-two"]);
    assert_eq!(find_closest_profile("axbxc", &p), None);
}
