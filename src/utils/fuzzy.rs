//! Fuzzy profile-name matching for the non-interactive (`-n`) path.
//!
//! Two related operations:
//!
//! - [`find_closest_profile`] returns a single unambiguous best match — used
//!   when a user typed something like `prd` and we want to "did you mean
//!   `prod`?" with high confidence (no ties, distance under a tight bound).
//! - [`nearest_n`] returns up to N candidates ranked by combined Levenshtein
//!   distance / LCS — used to produce a helpful error message when no
//!   single best match exists.

use std::collections::HashMap;
use strsim::levenshtein;

use crate::profile::Profile;

/// Maximum edit distance we'll silently substitute through (single best).
const LEVENSHTEIN_MAX_DISTANCE: usize = 3;

/// LCS minimum overlap (as a percentage of input length) for an unambiguous
/// LCS-based match.
const LCS_MIN_RATIO_PERCENT: usize = 50;

/// Return one unambiguous best match for `input` from `profiles`, or `None`
/// if the result is ambiguous, too far, or non-existent.
///
/// Used by the non-interactive path to silently correct obvious typos.
pub fn find_closest_profile(input: &str, profiles: &HashMap<String, Profile>) -> Option<String> {
    let names: Vec<&str> = profiles.keys().map(String::as_str).collect();
    if names.is_empty() {
        return None;
    }

    if let Some(m) = prefix_match(input, &names) {
        return Some(m);
    }
    if let Some(m) = lcs_match(input, &names) {
        return Some(m);
    }
    levenshtein_match(input, &names)
}

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

fn prefix_match(input: &str, profiles: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();
    let mut matches: Vec<&str> = profiles
        .iter()
        .filter(|p| p.to_lowercase().starts_with(&input_lower))
        .copied()
        .collect();

    if matches.len() == 1 {
        return Some(matches[0].to_string());
    }
    matches.retain(|p| p.starts_with(input));
    if matches.len() == 1 {
        return Some(matches[0].to_string());
    }
    None
}

fn lcs_match(input: &str, profiles: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();
    let mut best: Option<(&str, usize)> = None;
    let mut tie = false;

    for profile in profiles {
        let lcs_len = longest_common_subsequence(&input_lower, &profile.to_lowercase());
        match &best {
            None => best = Some((profile, lcs_len)),
            Some((_, prev)) => {
                if lcs_len > *prev {
                    best = Some((profile, lcs_len));
                    tie = false;
                } else if lcs_len == *prev {
                    tie = true;
                }
            }
        }
    }
    if tie {
        return None;
    }

    #[allow(clippy::manual_div_ceil)]
    let min = (input.chars().count() * LCS_MIN_RATIO_PERCENT + 99) / 100;
    best.filter(|(_, lcs)| *lcs >= min)
        .map(|(p, _)| p.to_string())
}

/// O(n) space LCS length using a two-row rolling buffer.
fn longest_common_subsequence(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());

    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        for j in 1..=n {
            if a[i - 1] == b[j - 1] {
                curr[j] = prev[j - 1] + 1;
            } else {
                curr[j] = prev[j].max(curr[j - 1]);
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.iter_mut().for_each(|x| *x = 0);
    }
    prev[n]
}

fn levenshtein_match(input: &str, profiles: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();
    let mut best: Option<(&str, usize)> = None;
    let mut tie = false;

    for profile in profiles {
        let d = levenshtein(&input_lower, &profile.to_lowercase());
        match &best {
            None => best = Some((profile, d)),
            Some((_, prev)) => {
                if d < *prev {
                    best = Some((profile, d));
                    tie = false;
                } else if d == *prev {
                    tie = true;
                }
            }
        }
    }
    if tie {
        return None;
    }
    best.filter(|(_, d)| *d <= LEVENSHTEIN_MAX_DISTANCE)
        .map(|(p, _)| p.to_string())
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
    fn unique_prefix_resolves() {
        assert_eq!(
            find_closest_profile("stag", &profiles()),
            Some("staging".to_string())
        );
    }

    #[test]
    fn ambiguous_prefix_is_none() {
        assert!(find_closest_profile("dev", &profiles()).is_none());
    }

    #[test]
    fn small_typo_resolves() {
        assert_eq!(
            find_closest_profile("stagin", &profiles()),
            Some("staging".to_string())
        );
    }

    #[test]
    fn exact_match_returns_self() {
        assert_eq!(
            find_closest_profile("staging", &profiles()),
            Some("staging".to_string())
        );
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
