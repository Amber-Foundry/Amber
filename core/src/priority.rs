// MindVault — Usage-Relative Priority (URP) System
//
// Nodes are not degraded or forgotten over time.
// A closed app is a frozen system — no state changes, no priority shifts.
// "MindVault Time" only advances when the user actively uses the app
// and generates touches on nodes.
//
// Vault-Relative Time: each vault has its own independent timeline.
// If vault A is active but vault B has no touches, vault B's nodes
// remain frozen — their access_history does not shift forward.

pub const DEFAULT_PRIORITY_JSON: &str = "{\"score\":1.0,\"profile\":\"standard\",\"pinned\":false,\"access_count_30active\":0,\"access_count_90active\":0,\"auto_trim_threshold\":0.25}";

/// Maximum score a priority profile can achieve.
fn max_score(profile: &str) -> f64 {
    match profile {
        "pinned" => 1.0,
        "slow" => 1.0,
        "fast" => 0.4,
        // "standard" and any unrecognized profile
        _ => 0.8,
    }
}

/// Calculate the Usage-Relative Priority score for a node.
///
/// Score is based entirely on usage frequency during active sessions.
/// Real-world time does not affect this calculation.
///
/// Formula:
///   if pinned → 1.0
///   base = (access_count_30active / 10.0).clamp(0.1, 1.0)
///   link_bonus = (link_count * 0.05).clamp(0.0, 0.2)
///   score = (base + link_bonus).min(profile.max_score())
pub fn calculate_score(access_count_30active: u64, link_count: u64, profile: &str) -> f64 {
    if profile == "pinned" {
        return 1.0;
    }

    let base = (access_count_30active as f64 / 10.0).clamp(0.1, 1.0);
    let link_bonus = (link_count as f64 * 0.05).clamp(0.0, 0.2);

    (base + link_bonus).min(max_score(profile))
}

const MAX_HISTORY_LEN: usize = 90;

/// Roll over daily access counters with vault-relative time.
///
/// `vault_is_active` — true if ANY node in this vault had touches today.
///
/// If `vault_is_active == false`, the vault is frozen: reset `today_touches`
/// to 0 but do NOT push anything to `access_history`. The node's history
/// and scores remain unchanged (winter-break / inactive-vault protection).
///
/// If `vault_is_active == true`, time is ticking for this vault: push
/// `today_touches` (even if 0) to the history array. This means untouched
/// nodes in an active vault naturally lose priority relative to their
/// siblings that ARE being accessed.
pub fn calculate_rollover(
    mut priority_json: serde_json::Value,
    vault_is_active: bool,
) -> serde_json::Value {
    let today = priority_json
        .get("today_touches")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Vault frozen — reset today_touches but do not advance history.
    if !vault_is_active {
        priority_json["today_touches"] = serde_json::json!(0);
        return priority_json;
    }

    // Vault is active — time is ticking. Push today_touches (even 0).
    let mut history: Vec<u64> = priority_json
        .get("access_history")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|item| item.as_u64()).collect())
        .unwrap_or_default();

    history.insert(0, today);
    history.truncate(MAX_HISTORY_LEN);

    let access_30d: u64 = history.iter().take(30).sum();
    let access_90d: u64 = history.iter().sum();

    priority_json["today_touches"] = serde_json::json!(0);
    priority_json["access_history"] = serde_json::json!(history);
    priority_json["access_count_30active"] = serde_json::json!(access_30d);
    priority_json["access_count_90active"] = serde_json::json!(access_90d);

    priority_json
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Score tests ──────────────────────────────────────────

    #[test]
    fn pinned_always_returns_one() {
        assert!((calculate_score(0, 0, "pinned") - 1.0).abs() < f64::EPSILON);
        assert!((calculate_score(100, 0, "pinned") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_access_returns_floor() {
        let score = calculate_score(0, 0, "standard");
        assert!(
            (score - 0.1).abs() < f64::EPSILON,
            "expected 0.1, got {score}"
        );
    }

    #[test]
    fn high_access_caps_at_max_score() {
        let standard = calculate_score(50, 0, "standard");
        assert!(
            (standard - 0.8).abs() < f64::EPSILON,
            "standard max should be 0.8, got {standard}"
        );

        let slow = calculate_score(50, 0, "slow");
        assert!(
            (slow - 1.0).abs() < f64::EPSILON,
            "slow max should be 1.0, got {slow}"
        );

        let fast = calculate_score(50, 0, "fast");
        assert!(
            (fast - 0.4).abs() < f64::EPSILON,
            "fast max should be 0.4, got {fast}"
        );
    }

    #[test]
    fn fast_scores_lower_than_slow_same_usage() {
        let fast = calculate_score(5, 0, "fast");
        let slow = calculate_score(5, 0, "slow");
        assert!(
            fast < slow,
            "fast ({fast}) should be less than slow ({slow})"
        );
    }

    #[test]
    fn link_bonus_adds_structural_signal() {
        let without = calculate_score(3, 0, "slow");
        let with = calculate_score(3, 4, "slow");
        assert!(
            with > without,
            "with links ({with}) should be greater than without ({without})"
        );
        assert!(
            (with - without - 0.2).abs() < f64::EPSILON,
            "link bonus should be 0.2, got {}",
            with - without
        );
    }

    #[test]
    fn link_bonus_capped_at_0_2() {
        let score = calculate_score(3, 100, "slow");
        assert!(
            (score - 0.5).abs() < f64::EPSILON,
            "expected 0.5, got {score}"
        );
    }

    // ── Rollover tests (vault active) ───────────────────────

    #[test]
    fn rollover_active_vault_pushes_today() {
        let input = serde_json::json!({
            "today_touches": 5,
            "access_history": [3, 2, 1]
        });
        let result = calculate_rollover(input, true);
        assert_eq!(result["today_touches"], 0);
        let history = result["access_history"].as_array().unwrap_or_else(|| {
            panic!("access_history missing");
        });
        assert_eq!(history[0], 5);
        assert_eq!(history[1], 3);
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn rollover_active_vault_pushes_zero_for_untouched_node() {
        // Node in an active vault that wasn't touched itself — 0 gets pushed
        let input = serde_json::json!({
            "today_touches": 0,
            "access_history": [5, 3]
        });
        let result = calculate_rollover(input, true);
        assert_eq!(result["today_touches"], 0);
        let history = result["access_history"].as_array().unwrap_or_else(|| {
            panic!("access_history missing");
        });
        assert_eq!(history.len(), 3, "0 should be pushed");
        assert_eq!(history[0], 0, "first entry should be 0");
        assert_eq!(history[1], 5);
    }

    #[test]
    fn rollover_active_vault_computes_30d_and_90d_sums() {
        let history = vec![1u64; 40];
        let input = serde_json::json!({
            "today_touches": 2,
            "access_history": history
        });
        let result = calculate_rollover(input, true);
        assert_eq!(result["access_count_30active"], 31);
        // 90 active-session sum = 2 + 40*1 = 42
        assert_eq!(result["access_count_90active"], 42);
    }

    #[test]
    fn rollover_active_vault_caps_history_at_90() {
        let history = vec![1u64; 95];
        let input = serde_json::json!({
            "today_touches": 1,
            "access_history": history
        });
        let result = calculate_rollover(input, true);
        let arr = result["access_history"].as_array().unwrap_or_else(|| {
            panic!("access_history missing");
        });
        assert_eq!(arr.len(), 90);
    }

    // ── Rollover tests (vault frozen) ───────────────────────

    #[test]
    fn rollover_frozen_vault_does_not_shift_history() {
        let input = serde_json::json!({
            "today_touches": 3,
            "access_history": [5, 3],
            "access_count_30active": 8,
            "access_count_90active": 8
        });
        let result = calculate_rollover(input, false);
        // today_touches is reset but history is NOT shifted
        assert_eq!(result["today_touches"], 0);
        let history = result["access_history"].as_array().unwrap_or_else(|| {
            panic!("access_history missing");
        });
        assert_eq!(history.len(), 2, "history should be unchanged");
        assert_eq!(
            result["access_count_30active"], 8,
            "30active should be unchanged"
        );
        assert_eq!(
            result["access_count_90active"], 8,
            "90active should be unchanged"
        );
    }

    #[test]
    fn rollover_frozen_vault_resets_today_touches() {
        let input = serde_json::json!({
            "today_touches": 7
        });
        let result = calculate_rollover(input, false);
        assert_eq!(result["today_touches"], 0);
        assert!(
            result.get("access_history").is_none(),
            "should not create history on frozen vault"
        );
    }

    #[test]
    fn rollover_active_vault_handles_empty_defaults() {
        let input = serde_json::json!({});
        let result = calculate_rollover(input, true);
        assert_eq!(result["today_touches"], 0);
        let history = result["access_history"].as_array().unwrap_or_else(|| {
            panic!("access_history missing");
        });
        assert_eq!(history.len(), 1, "should push 0 for active vault");
        assert_eq!(history[0], 0);
    }
}
