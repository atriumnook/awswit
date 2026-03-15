use std::collections::HashMap;
use strsim::levenshtein;

use crate::profile::Profile;

const LEVENSHTEIN_MAX_DISTANCE: usize = 3;
const LCS_MIN_RATIO_PERCENT: usize = 50;

/// Find the closest matching profile name using fuzzy matching
///
/// Uses three methods in order:
/// 1. Prefix matching
/// 2. Longest common subsequence
/// 3. Levenshtein distance
pub fn find_closest_profile(input: &str, profiles: &HashMap<String, Profile>) -> Option<String> {
    let profile_names: Vec<&str> = profiles.keys().map(|s| s.as_str()).collect();

    if profile_names.is_empty() {
        return None;
    }

    // Try prefix matching first
    if let Some(matched) = prefix_match(input, &profile_names) {
        tracing::debug!("Fuzzy matched '{}' using prefix match", matched);
        return Some(matched);
    }

    // Try longest common subsequence
    if let Some(matched) = lcs_match(input, &profile_names) {
        tracing::debug!("Fuzzy matched '{}' using LCS", matched);
        return Some(matched);
    }

    // Try Levenshtein distance
    if let Some(matched) = levenshtein_match(input, &profile_names) {
        tracing::debug!("Fuzzy matched '{}' using Levenshtein", matched);
        return Some(matched);
    }

    None
}

/// Match profiles by prefix
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

    // If multiple matches, try exact prefix match
    matches.retain(|p| p.starts_with(input));
    if matches.len() == 1 {
        return Some(matches[0].to_string());
    }

    None
}

/// Match profiles using longest common subsequence
fn lcs_match(input: &str, profiles: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();

    let mut best_match: Option<(&str, usize)> = None;
    let mut is_tie = false;

    for profile in profiles {
        let profile_lower = profile.to_lowercase();
        let lcs_len = longest_common_subsequence(&input_lower, &profile_lower);

        match &best_match {
            None => {
                best_match = Some((profile, lcs_len));
            }
            Some((_, best_len)) => {
                if lcs_len > *best_len {
                    best_match = Some((profile, lcs_len));
                    is_tie = false;
                } else if lcs_len == *best_len {
                    is_tie = true;
                }
            }
        }
    }

    if is_tie {
        return None;
    }

    // Require LCS length to be at least LCS_MIN_RATIO_PERCENT% of input length.
    #[allow(clippy::manual_div_ceil)]
    let min_lcs = (input.chars().count() * LCS_MIN_RATIO_PERCENT + 99) / 100;
    best_match
        .filter(|(_, lcs_len)| *lcs_len >= min_lcs)
        .map(|(p, _)| p.to_string())
}

/// Calculate longest common subsequence length.
/// Uses two-row rolling array for O(n) space instead of O(m*n).
fn longest_common_subsequence(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let m = a_chars.len();
    let n = b_chars.len();

    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        for j in 1..=n {
            if a_chars[i - 1] == b_chars[j - 1] {
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

/// Match profiles using Levenshtein distance
fn levenshtein_match(input: &str, profiles: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();

    let mut best_match: Option<(&str, usize)> = None;
    let mut is_tie = false;

    for profile in profiles {
        let profile_lower = profile.to_lowercase();
        let distance = levenshtein(&input_lower, &profile_lower);

        match &best_match {
            None => {
                best_match = Some((profile, distance));
            }
            Some((_, best_dist)) => {
                if distance < *best_dist {
                    best_match = Some((profile, distance));
                    is_tie = false;
                } else if distance == *best_dist {
                    is_tie = true;
                }
            }
        }
    }

    // Only accept if distance is reasonable (e.g., < 3 for small typos)
    if is_tie {
        return None;
    }

    best_match
        .filter(|(_, dist)| *dist <= LEVENSHTEIN_MAX_DISTANCE)
        .map(|(p, _)| p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_profiles() -> HashMap<String, Profile> {
        let mut profiles = HashMap::new();
        profiles.insert("dev-admin".to_string(), Profile::default());
        profiles.insert("dev-readonly".to_string(), Profile::default());
        profiles.insert("prod-admin".to_string(), Profile::default());
        profiles.insert("staging".to_string(), Profile::default());
        profiles
    }

    #[test]
    fn test_prefix_match() {
        let profiles = create_test_profiles();

        // Unique prefix
        let result = find_closest_profile("stag", &profiles);
        assert_eq!(result, Some("staging".to_string()));

        // Ambiguous prefix (dev- matches multiple)
        let result = find_closest_profile("dev", &profiles);
        // "dev" is ambiguous prefix (matches dev-admin and dev-readonly), so no match
        assert!(result.is_none());
    }

    #[test]
    fn test_typo_match() {
        let profiles = create_test_profiles();

        // Small typo
        let result = find_closest_profile("stagin", &profiles);
        assert_eq!(result, Some("staging".to_string()));

        // Transposition
        let result = find_closest_profile("stagign", &profiles);
        assert_eq!(result, Some("staging".to_string()));
    }

    #[test]
    fn test_lcs() {
        assert_eq!(longest_common_subsequence("abc", "abc"), 3);
        assert_eq!(longest_common_subsequence("abc", "def"), 0);
        assert_eq!(longest_common_subsequence("abc", "adc"), 2);
    }

    #[test]
    fn test_exact_match() {
        let profiles = create_test_profiles();

        // Exact match should be preferred
        let result = find_closest_profile("staging", &profiles);
        assert_eq!(result, Some("staging".to_string()));
    }
}
