//! "Did you mean…?" suggestions for unknown profile names.
//!
//! These only ever feed an error message. awswit never substitutes a
//! suggested profile for the one the user named: profiles map to accounts,
//! so a guess can switch the shell to the wrong one.

use std::collections::HashMap;
use strsim::levenshtein;

use crate::profile::Profile;

/// Return up to `n` candidates from `profiles` ranked by Levenshtein
/// distance ascending (closest first). Ties are broken alphabetically so
/// suggestions are deterministic across runs.
///
/// Suitable for "Profile not found — did you mean…?" hints.
pub fn nearest_n(input: &str, profiles: &HashMap<String, Profile>, n: usize) -> Vec<String> {
    let input_lower = input.to_lowercase();
    let mut scored: Vec<(usize, &str)> = profiles
        .keys()
        .map(|name| {
            let lower = name.to_lowercase();
            (levenshtein(&input_lower, &lower), name.as_str())
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(n)
        .map(|(_, name)| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles() -> HashMap<String, Profile> {
        ["dev-admin", "dev-readonly", "prod-admin", "staging"]
            .into_iter()
            .map(|n| (n.to_string(), Profile::default()))
            .collect()
    }

    #[test]
    fn nearest_n_returns_ranked_candidates() {
        let near = nearest_n("dev", &profiles(), 3);
        assert!(near.contains(&"dev-admin".to_string()));
        assert!(near.contains(&"dev-readonly".to_string()));
        assert!(near.len() <= 3);
    }

    #[test]
    fn nearest_n_empty_input_returns_lowest_distance() {
        let near = nearest_n("staging", &profiles(), 1);
        assert_eq!(near, vec!["staging".to_string()]);
    }
}
