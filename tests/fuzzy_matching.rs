//! Fuzzy matching tests using the real awswit library

use std::collections::HashMap;
use awswit::profile::Profile;
use awswit::utils::fuzzy::find_closest_profile;

fn profiles(names: &[&str]) -> HashMap<String, Profile> {
    names.iter().map(|n| (n.to_string(), Profile::default())).collect()
}

#[test]
fn test_fuzzy_exact_match() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(find_closest_profile("dev-admin", &p), Some("dev-admin".to_string()));
}

#[test]
fn test_fuzzy_prefix_match() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(find_closest_profile("stag", &p), Some("staging".to_string()));
}

#[test]
fn test_fuzzy_typo() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(find_closest_profile("stagin", &p), Some("staging".to_string()));
}

#[test]
fn test_fuzzy_completely_different() {
    let p = profiles(&["dev-admin", "staging"]);
    assert_eq!(find_closest_profile("zzzzzzzzzzzzz", &p), None);
}
