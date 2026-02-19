use awswit::utils::fuzzy::{filter_profiles, fuzzy_match};

#[test]
fn test_exact_match_priority() {
    let candidates = vec![
        "dev".to_string(),
        "dev-admin".to_string(),
        "development".to_string(),
        "prod".to_string(),
    ];

    let matches = fuzzy_match("dev", &candidates);
    assert!(!matches.is_empty());
    // Exact match should be first
    assert_eq!(matches[0].name, "dev");
}

#[test]
fn test_prefix_match_priority() {
    let candidates = vec![
        "prod-admin".to_string(),
        "my-dev-account".to_string(),
        "dev-staging".to_string(),
    ];

    let matches = fuzzy_match("dev", &candidates);
    assert!(!matches.is_empty());
    // Prefix match should come before substring match
    assert_eq!(matches[0].name, "dev-staging");
}

#[test]
fn test_fuzzy_with_separators() {
    let candidates = vec![
        "prod-admin-readonly".to_string(),
        "prod-developer".to_string(),
        "staging-admin".to_string(),
    ];

    let matches = fuzzy_match("prod-admin", &candidates);
    assert!(!matches.is_empty());
    assert_eq!(matches[0].name, "prod-admin-readonly");
}

#[test]
fn test_case_insensitive_matching() {
    let candidates = vec![
        "MyDevAccount".to_string(),
        "PRODUCTION".to_string(),
        "staging".to_string(),
    ];

    let matches = fuzzy_match("mydev", &candidates);
    assert!(!matches.is_empty());
    assert_eq!(matches[0].name, "MyDevAccount");
}

#[test]
fn test_subsequence_matching() {
    let candidates = vec![
        "prod-admin".to_string(),
        "staging".to_string(),
        "dev".to_string(),
    ];

    // "padm" is a subsequence of "prod-admin"
    let matches = fuzzy_match("padm", &candidates);
    assert!(!matches.is_empty());
    assert_eq!(matches[0].name, "prod-admin");
}

#[test]
fn test_no_match_returns_empty() {
    let candidates = vec!["dev".to_string(), "prod".to_string()];

    let matches = fuzzy_match("zzzzzzz", &candidates);
    assert!(matches.is_empty());
}

#[test]
fn test_empty_query_returns_all() {
    let candidates = vec![
        "dev".to_string(),
        "prod".to_string(),
        "staging".to_string(),
    ];

    let matches = fuzzy_match("", &candidates);
    assert_eq!(matches.len(), 3);
}

#[test]
fn test_filter_profiles_with_fuzzy() {
    let profiles = vec![
        "default".to_string(),
        "dev".to_string(),
        "prod-admin".to_string(),
        "staging".to_string(),
    ];

    let filtered = filter_profiles("dev", &profiles, true);
    assert!(filtered.contains(&"dev".to_string()));
    // Should not contain unrelated profiles
    assert!(!filtered.contains(&"staging".to_string()));
}

#[test]
fn test_filter_profiles_without_fuzzy() {
    let profiles = vec![
        "default".to_string(),
        "dev".to_string(),
        "dev-staging".to_string(),
        "prod".to_string(),
    ];

    let filtered = filter_profiles("dev", &profiles, false);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.contains(&"dev".to_string()));
    assert!(filtered.contains(&"dev-staging".to_string()));
}

#[test]
fn test_filter_empty_query() {
    let profiles = vec!["dev".to_string(), "prod".to_string()];
    let filtered = filter_profiles("", &profiles, true);
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_typo_tolerance() {
    let candidates = vec![
        "development".to_string(),
        "production".to_string(),
        "staging".to_string(),
    ];

    // "developmnet" is a typo for "development"
    let matches = fuzzy_match("developmnet", &candidates);
    // Should still match via Jaro-Winkler similarity
    assert!(!matches.is_empty());
    assert_eq!(matches[0].name, "development");
}

#[test]
fn test_many_profiles() {
    let candidates: Vec<String> = (0..100).map(|i| format!("profile-{:03}", i)).collect();

    let matches = fuzzy_match("profile-042", &candidates);
    assert!(!matches.is_empty());
    assert_eq!(matches[0].name, "profile-042");
}
