/// Fuzzy matching implementation for profile names
///
/// Uses a combination of substring matching and edit distance for flexible matching.
use strsim::jaro_winkler;

/// Result of a fuzzy match
#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    pub name: String,
    pub score: f64,
}

/// Perform fuzzy matching of a query against a list of candidates
pub fn fuzzy_match(query: &str, candidates: &[String]) -> Vec<FuzzyMatch> {
    if query.is_empty() {
        return candidates
            .iter()
            .map(|c| FuzzyMatch {
                name: c.clone(),
                score: 1.0,
            })
            .collect();
    }

    let query_lower = query.to_lowercase();
    let mut matches: Vec<FuzzyMatch> = Vec::new();

    for candidate in candidates {
        let candidate_lower = candidate.to_lowercase();

        // Exact match gets highest score
        if candidate_lower == query_lower {
            matches.push(FuzzyMatch {
                name: candidate.clone(),
                score: 2.0,
            });
            continue;
        }

        // Prefix match gets high score
        if candidate_lower.starts_with(&query_lower) {
            matches.push(FuzzyMatch {
                name: candidate.clone(),
                score: 1.5 + (query.len() as f64 / candidate.len() as f64) * 0.5,
            });
            continue;
        }

        // Substring match
        if candidate_lower.contains(&query_lower) {
            matches.push(FuzzyMatch {
                name: candidate.clone(),
                score: 1.0 + (query.len() as f64 / candidate.len() as f64) * 0.5,
            });
            continue;
        }

        // Subsequence match (characters appear in order)
        if is_subsequence(&query_lower, &candidate_lower) {
            let score = subsequence_score(&query_lower, &candidate_lower);
            if score > 0.3 {
                matches.push(FuzzyMatch {
                    name: candidate.clone(),
                    score,
                });
                continue;
            }
        }

        // Jaro-Winkler similarity for typo tolerance
        let jw_score = jaro_winkler(&query_lower, &candidate_lower);
        if jw_score > 0.7 {
            matches.push(FuzzyMatch {
                name: candidate.clone(),
                score: jw_score * 0.8,
            });
        }
    }

    matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    matches
}

/// Check if query chars appear in order within candidate
fn is_subsequence(query: &str, candidate: &str) -> bool {
    let mut query_chars = query.chars();
    let mut current = query_chars.next();

    for c in candidate.chars() {
        if let Some(q) = current {
            if c == q {
                current = query_chars.next();
            }
        } else {
            return true;
        }
    }

    current.is_none()
}

/// Score a subsequence match based on character positions and gaps
fn subsequence_score(query: &str, candidate: &str) -> f64 {
    let query_chars: Vec<char> = query.chars().collect();
    let candidate_chars: Vec<char> = candidate.chars().collect();

    if query_chars.is_empty() || candidate_chars.is_empty() {
        return 0.0;
    }

    let mut qi = 0;
    let mut total_gap = 0;
    let mut prev_pos: Option<usize> = None;
    let mut consecutive = 0;

    for (ci, &c) in candidate_chars.iter().enumerate() {
        if qi < query_chars.len() && c == query_chars[qi] {
            if let Some(pp) = prev_pos {
                let gap = ci - pp - 1;
                total_gap += gap;
                if gap == 0 {
                    consecutive += 1;
                }
            }
            prev_pos = Some(ci);
            qi += 1;
        }
    }

    if qi < query_chars.len() {
        return 0.0;
    }

    let coverage = query_chars.len() as f64 / candidate_chars.len() as f64;
    let gap_penalty = if query_chars.len() > 1 {
        1.0 - (total_gap as f64 / candidate_chars.len() as f64).min(1.0)
    } else {
        1.0
    };
    let consecutive_bonus = consecutive as f64 / query_chars.len().max(1) as f64;

    (coverage * 0.4 + gap_penalty * 0.4 + consecutive_bonus * 0.2).min(1.0)
}

/// Filter profiles by exact or fuzzy match, returning matched names in order
pub fn filter_profiles(query: &str, profiles: &[String], fuzzy_enabled: bool) -> Vec<String> {
    if query.is_empty() {
        return profiles.to_vec();
    }

    if fuzzy_enabled {
        fuzzy_match(query, profiles)
            .into_iter()
            .map(|m| m.name)
            .collect()
    } else {
        // Simple prefix/substring filter
        let query_lower = query.to_lowercase();
        profiles
            .iter()
            .filter(|p| p.to_lowercase().contains(&query_lower))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let candidates = vec!["dev".to_string(), "prod".to_string(), "staging".to_string()];
        let matches = fuzzy_match("dev", &candidates);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "dev");
    }

    #[test]
    fn test_prefix_match() {
        let candidates = vec![
            "dev-account".to_string(),
            "prod".to_string(),
            "staging".to_string(),
        ];
        let matches = fuzzy_match("dev", &candidates);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "dev-account");
    }

    #[test]
    fn test_substring_match() {
        let candidates = vec![
            "my-dev-account".to_string(),
            "prod".to_string(),
            "staging".to_string(),
        ];
        let matches = fuzzy_match("dev", &candidates);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "my-dev-account");
    }

    #[test]
    fn test_subsequence_match() {
        let candidates = vec![
            "prod-admin".to_string(),
            "staging".to_string(),
            "dev".to_string(),
        ];
        let matches = fuzzy_match("padm", &candidates);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "prod-admin");
    }

    #[test]
    fn test_empty_query() {
        let candidates = vec!["dev".to_string(), "prod".to_string()];
        let matches = fuzzy_match("", &candidates);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_no_match() {
        let candidates = vec!["dev".to_string(), "prod".to_string()];
        let matches = fuzzy_match("zzzzzzz", &candidates);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_case_insensitive() {
        let candidates = vec!["DevAccount".to_string()];
        let matches = fuzzy_match("dev", &candidates);
        assert!(!matches.is_empty());
    }

    #[test]
    fn test_filter_profiles_fuzzy() {
        let profiles = vec![
            "dev".to_string(),
            "prod".to_string(),
            "staging".to_string(),
        ];
        let filtered = filter_profiles("dev", &profiles, true);
        assert_eq!(filtered[0], "dev");
    }

    #[test]
    fn test_filter_profiles_no_fuzzy() {
        let profiles = vec![
            "dev".to_string(),
            "prod".to_string(),
            "dev-staging".to_string(),
        ];
        let filtered = filter_profiles("dev", &profiles, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&"dev".to_string()));
        assert!(filtered.contains(&"dev-staging".to_string()));
    }

    #[test]
    fn test_is_subsequence() {
        assert!(is_subsequence("abc", "aXbXc"));
        assert!(is_subsequence("abc", "abc"));
        assert!(!is_subsequence("abc", "acb"));
        assert!(is_subsequence("", "anything"));
    }
}
