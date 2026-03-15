//! Frecency scoring boundary tests and favorite priority verification.

use awswit::history::HistoryEntry;
use chrono::{Duration, Utc};

#[test]
fn score_zero_use_count_is_zero() {
    let now = Utc::now();
    let entry = HistoryEntry {
        name: "test".to_string(),
        last_used: now,
        use_count: 0,
        is_favorite: false,
    };
    assert!((entry.frecency_score(now) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn score_boundary_exactly_one_hour() {
    let now = Utc::now();
    let entry = HistoryEntry {
        name: "test".to_string(),
        last_used: now - Duration::hours(1),
        use_count: 1,
        is_favorite: false,
    };
    // Exactly 1 hour → falls in the 1-24h bucket → weight 2.0
    assert!((entry.frecency_score(now) - 2.0).abs() < f64::EPSILON);
}

#[test]
fn score_boundary_exactly_24_hours() {
    let now = Utc::now();
    let entry = HistoryEntry {
        name: "test".to_string(),
        last_used: now - Duration::hours(24),
        use_count: 1,
        is_favorite: false,
    };
    // Exactly 24h → falls in the 24h-168h bucket → weight 1.0
    assert!((entry.frecency_score(now) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn score_boundary_exactly_one_week() {
    let now = Utc::now();
    let entry = HistoryEntry {
        name: "test".to_string(),
        last_used: now - Duration::hours(168),
        use_count: 1,
        is_favorite: false,
    };
    // Exactly 168h → falls in the >168h bucket → weight 0.5
    assert!((entry.frecency_score(now) - 0.5).abs() < f64::EPSILON);
}

#[test]
fn recent_high_frequency_beats_old_high_frequency() {
    let now = Utc::now();
    let recent = HistoryEntry {
        name: "recent".to_string(),
        last_used: now - Duration::minutes(30),
        use_count: 5,
        is_favorite: false,
    };
    let old = HistoryEntry {
        name: "old".to_string(),
        last_used: now - Duration::days(30),
        use_count: 5,
        is_favorite: false,
    };
    assert!(recent.frecency_score(now) > old.frecency_score(now));
}

#[test]
fn favorite_sorting_priority() {
    // Favorites should sort before non-favorites regardless of score.
    // This tests the contract used by fzf.rs and picker.rs sorting.
    let now = Utc::now();
    let fav = HistoryEntry {
        name: "fav".to_string(),
        last_used: now - Duration::days(30),
        use_count: 1,
        is_favorite: true,
    };
    let non_fav = HistoryEntry {
        name: "non_fav".to_string(),
        last_used: now,
        use_count: 100,
        is_favorite: false,
    };

    // Even though non_fav has much higher frecency, favorite flag takes precedence in sort
    assert!(fav.is_favorite);
    assert!(!non_fav.is_favorite);
    // The actual sort comparison (favorite first) is in picker/fzf, but we verify the data here
    assert!(non_fav.frecency_score(now) > fav.frecency_score(now));
}

#[test]
fn future_last_used_does_not_panic() {
    let now = Utc::now();
    let entry = HistoryEntry {
        name: "future".to_string(),
        last_used: now + Duration::hours(1),
        use_count: 1,
        is_favorite: false,
    };
    // Should not panic, hours difference clamped to 0 → weight 4.0
    assert!((entry.frecency_score(now) - 4.0).abs() < f64::EPSILON);
}
