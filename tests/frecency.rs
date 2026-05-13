//! Frecency scoring properties under the continuous-decay model.
//!
//! Score = `use_count * 2^(-age_hours / 72)`. We verify monotonicity, the
//! halving property at the half-life boundary, and edge cases rather than
//! pinning specific numeric values which would over-constrain future tuning.

use awswit::history::HistoryEntry;
use chrono::{DateTime, Duration, Utc};

fn entry(hours_ago: i64, use_count: u32) -> HistoryEntry {
    HistoryEntry {
        name: "p".into(),
        last_used: Utc::now() - Duration::hours(hours_ago),
        use_count,
        is_favorite: false,
    }
}

fn score(e: &HistoryEntry) -> f64 {
    e.frecency_score(Utc::now())
}

#[test]
fn zero_use_count_scores_zero() {
    let e = HistoryEntry {
        name: "p".into(),
        last_used: Utc::now(),
        use_count: 0,
        is_favorite: false,
    };
    assert_eq!(score(&e), 0.0);
}

#[test]
fn score_is_monotonically_decreasing_in_age() {
    let s0 = score(&entry(0, 1));
    let s1 = score(&entry(1, 1));
    let s24 = score(&entry(24, 1));
    let s168 = score(&entry(168, 1));
    let s720 = score(&entry(720, 1));
    assert!(s0 > s1, "{} not > {}", s0, s1);
    assert!(s1 > s24);
    assert!(s24 > s168);
    assert!(s168 > s720);
    assert!(s720 > 0.0);
}

#[test]
fn score_halves_at_72_hour_half_life() {
    let fresh = score(&entry(0, 10));
    let halved = score(&entry(72, 10));
    // ~5% tolerance to absorb timing jitter between Utc::now() calls.
    assert!(
        (halved - fresh / 2.0).abs() / fresh < 0.05,
        "fresh={}, halved={}",
        fresh,
        halved
    );
}

#[test]
fn score_scales_linearly_with_use_count() {
    let one = score(&entry(10, 1));
    let ten = score(&entry(10, 10));
    assert!((ten - 10.0 * one).abs() < 1e-9);
}

#[test]
fn recent_use_outranks_old_use_at_same_frequency() {
    let now: DateTime<Utc> = Utc::now();
    let recent = HistoryEntry {
        name: "recent".into(),
        last_used: now - Duration::minutes(30),
        use_count: 5,
        is_favorite: false,
    };
    let old = HistoryEntry {
        name: "old".into(),
        last_used: now - Duration::days(30),
        use_count: 5,
        is_favorite: false,
    };
    assert!(recent.frecency_score(now) > old.frecency_score(now));
}

#[test]
fn future_timestamps_are_clamped_to_now() {
    let now = Utc::now();
    let future = HistoryEntry {
        name: "future".into(),
        last_used: now + Duration::hours(1),
        use_count: 1,
        is_favorite: false,
    };
    // No panic, score equals fresh-now score (clamped age = 0).
    assert!((future.frecency_score(now) - 1.0).abs() < 1e-9);
}
