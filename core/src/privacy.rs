//! Amber privacy tiers — three simplified tiers, two independent axes.
//!
//! ## Axis 1: Egress (where may AI see this?)
//!
//! | Tier        | Cloud LLM | Local LLM (device)      |
//! |-------------|-----------|-------------------------|
//! | `open`      | full      | full                    |
//! | `local_only`| omit      | full                    |
//! | `redacted`  | omit      | full when session unlocked; omit otherwise |
//!
//! ## Axis 2: Disclosure (what is visible before unlock?)
//!
//! | Tier        | UI metadata (title, nav) | Body content | At-rest encryption |
//! |-------------|----------------------------|--------------|--------------------|
//! | `open`      | visible                    | visible      | no                 |
//! | `local_only`| visible                    | visible      | no                 |
//! | `redacted`  | hidden (`[REDACTED]`)      | gated        | yes                |
//!
//! ## Durable indexes (embeddings, exports)
//!
//! Persisted vectors must not store cleartext that the tier withholds from cloud context.
//!
//! | Tier        | Embedding policy                          |
//! |-------------|-------------------------------------------|
//! | `open`      | full cleartext                            |
//! | `local_only`| full cleartext (local ONNX / local Ollama)|
//! | `redacted`  | skip (delete existing vectors)            |
//!
//! Effective tier resolves as the strictest of node, sub-vault, and vault tiers.
//! All subsystems (`llm::assembler`, `embed::job`, UI helpers) follow this matrix.

pub const TIER_OPEN: &str = "open";
pub const TIER_LOCAL_ONLY: &str = "local_only";
pub const TIER_REDACTED: &str = "redacted";

const OPEN: &str = TIER_OPEN;
const LOCAL_ONLY: &str = TIER_LOCAL_ONLY;
const REDACTED: &str = TIER_REDACTED;

/// Whether full node content may be sent to a cloud LLM.
pub fn allows_cloud_content(tier: &str) -> bool {
    normalize_tier(Some(tier)) == OPEN
}

/// Whether node/vault payload is encrypted at rest (`encrypted_payload`).
/// Disclosure axis — use when persisting redacted tier content.
#[allow(dead_code)]
pub fn encrypts_at_rest(tier: &str) -> bool {
    normalize_tier(Some(tier)) == REDACTED
}

/// Whether UI should hide metadata until the master-password session is active.
/// Disclosure axis — mirror in `ui/utils/privacy.ts` display helpers.
#[allow(dead_code)]
pub fn hides_metadata_until_unlock(tier: &str) -> bool {
    normalize_tier(Some(tier)) == REDACTED
}

/// Whether embeddings should be skipped and any existing vectors deleted.
pub fn embedding_should_skip(tier: &str) -> bool {
    normalize_tier(Some(tier)) == REDACTED
}

/// Whether a node must be omitted from local LLM context assembly when locked.
pub fn omits_from_local_llm(tier: &str, is_unlocked: bool) -> bool {
    normalize_tier(Some(tier)) == REDACTED && !is_unlocked
}

/// Whether local-only nodes must not embed via a non-loopback Ollama endpoint.
pub fn embedding_blocks_on_remote_ollama(tier: &str) -> bool {
    normalize_tier(Some(tier)) == LOCAL_ONLY
}

/// How a node should appear in assembled LLM context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmContextPolicy {
    Full,
    Omit,
}

pub fn cloud_llm_context_policy(tier: &str) -> LlmContextPolicy {
    if allows_cloud_content(tier) {
        LlmContextPolicy::Full
    } else {
        LlmContextPolicy::Omit
    }
}

pub fn local_llm_context_policy(tier: &str, is_unlocked: bool) -> LlmContextPolicy {
    if omits_from_local_llm(tier, is_unlocked) {
        LlmContextPolicy::Omit
    } else {
        LlmContextPolicy::Full
    }
}

/// Unrestricted/debug scopes: full content for non-redacted tiers.
pub fn unrestricted_llm_context_policy(tier: &str) -> LlmContextPolicy {
    if normalize_tier(Some(tier)) == REDACTED {
        LlmContextPolicy::Omit
    } else {
        LlmContextPolicy::Full
    }
}

fn normalize_tier(tier: Option<&str>) -> &'static str {
    match tier {
        Some(LOCAL_ONLY) => LOCAL_ONLY,
        Some(REDACTED) => REDACTED,
        _ => OPEN,
    }
}

pub fn get_privacy_rank(tier: Option<&str>) -> u8 {
    match normalize_tier(tier) {
        OPEN => 0,
        LOCAL_ONLY => 1,
        REDACTED => 2,
        _ => 0,
    }
}

pub fn resolve_chain_effective_privacy<'a>(
    tiers: impl IntoIterator<Item = Option<&'a str>>,
) -> &'static str {
    let mut strictest = OPEN;
    for tier in tiers {
        let norm = normalize_tier(tier);
        if get_privacy_rank(Some(norm)) > get_privacy_rank(Some(strictest)) {
            strictest = norm;
        }
    }
    strictest
}

pub fn get_effective_privacy(
    node_tier: Option<&str>,
    sub_vault_tier: Option<&str>,
    vault_tier: Option<&str>,
) -> &'static str {
    resolve_chain_effective_privacy([node_tier, sub_vault_tier, vault_tier])
}

#[cfg(test)]
mod tests {
    use super::{
        allows_cloud_content, cloud_llm_context_policy, embedding_blocks_on_remote_ollama,
        embedding_should_skip, encrypts_at_rest, get_effective_privacy, get_privacy_rank,
        hides_metadata_until_unlock, local_llm_context_policy, omits_from_local_llm,
        unrestricted_llm_context_policy, LlmContextPolicy, TIER_LOCAL_ONLY, TIER_OPEN,
        TIER_REDACTED,
    };

    #[test]
    fn privacy_waterfall_parent_local_only_beats_node_open() {
        let effective = get_effective_privacy(Some("open"), Some("local_only"), None);
        assert_eq!(effective, "local_only");
    }

    #[test]
    fn privacy_waterfall_node_redacted_beats_parent_open() {
        let effective = get_effective_privacy(Some("redacted"), Some("open"), None);
        assert_eq!(effective, "redacted");
    }

    #[test]
    fn privacy_waterfall_same_tier_stays_same() {
        let effective = get_effective_privacy(Some("local_only"), Some("local_only"), None);
        assert_eq!(effective, "local_only");
    }

    #[test]
    fn privacy_waterfall_redacted_beats_local_only_across_hierarchy() {
        let effective = get_effective_privacy(Some("open"), Some("local_only"), Some("redacted"));
        assert_eq!(effective, "redacted");
    }

    #[test]
    fn privacy_rank_unknown_tiers_fall_back_to_open() {
        assert_eq!(get_privacy_rank(Some("unknown")), 0);
        let effective = get_effective_privacy(Some("mystery"), Some("still-unknown"), None);
        assert_eq!(effective, "open");
    }

    #[test]
    fn egress_policy_matrix() {
        assert!(allows_cloud_content(TIER_OPEN));
        assert!(!allows_cloud_content(TIER_LOCAL_ONLY));
        assert!(!allows_cloud_content(TIER_REDACTED));
        assert_eq!(
            cloud_llm_context_policy(TIER_LOCAL_ONLY),
            LlmContextPolicy::Omit
        );
        assert_eq!(
            cloud_llm_context_policy(TIER_REDACTED),
            LlmContextPolicy::Omit
        );
        assert_eq!(cloud_llm_context_policy(TIER_OPEN), LlmContextPolicy::Full);
    }

    #[test]
    fn disclosure_and_embedding_policy_matrix() {
        assert!(encrypts_at_rest(TIER_REDACTED));
        assert!(!encrypts_at_rest(TIER_LOCAL_ONLY));
        assert!(hides_metadata_until_unlock(TIER_REDACTED));
        assert!(!hides_metadata_until_unlock(TIER_LOCAL_ONLY));
        assert!(embedding_should_skip(TIER_REDACTED));
        assert!(!embedding_should_skip(TIER_LOCAL_ONLY));
    }

    #[test]
    fn llm_context_policy_matrix() {
        assert_eq!(cloud_llm_context_policy(TIER_OPEN), LlmContextPolicy::Full);
        assert_eq!(
            cloud_llm_context_policy(TIER_LOCAL_ONLY),
            LlmContextPolicy::Omit
        );
        assert_eq!(
            local_llm_context_policy(TIER_REDACTED, false),
            LlmContextPolicy::Omit
        );
        assert_eq!(
            local_llm_context_policy(TIER_REDACTED, true),
            LlmContextPolicy::Full
        );
        assert_eq!(
            local_llm_context_policy(TIER_LOCAL_ONLY, false),
            LlmContextPolicy::Full
        );
        assert_eq!(
            unrestricted_llm_context_policy(TIER_REDACTED),
            LlmContextPolicy::Omit
        );
        assert!(omits_from_local_llm(TIER_REDACTED, false));
        assert!(!omits_from_local_llm(TIER_REDACTED, true));
        assert!(embedding_blocks_on_remote_ollama(TIER_LOCAL_ONLY));
    }

    #[test]
    fn commit_2_privacy_decision_table_matrix() {
        use super::resolve_chain_effective_privacy;

        // Matrix tests: 3 tiers (Ancestor Tier, Descendant Tier) -> Expected Effective Tier
        let cases = [
            (Some(TIER_OPEN), Some(TIER_OPEN), TIER_OPEN),
            (Some(TIER_OPEN), Some(TIER_LOCAL_ONLY), TIER_LOCAL_ONLY),
            (Some(TIER_OPEN), Some(TIER_REDACTED), TIER_REDACTED),
            (Some(TIER_LOCAL_ONLY), Some(TIER_OPEN), TIER_LOCAL_ONLY),
            (
                Some(TIER_LOCAL_ONLY),
                Some(TIER_LOCAL_ONLY),
                TIER_LOCAL_ONLY,
            ),
            (Some(TIER_LOCAL_ONLY), Some(TIER_REDACTED), TIER_REDACTED),
            (Some(TIER_REDACTED), Some(TIER_OPEN), TIER_REDACTED),
            (Some(TIER_REDACTED), Some(TIER_LOCAL_ONLY), TIER_REDACTED),
            (Some(TIER_REDACTED), Some(TIER_REDACTED), TIER_REDACTED),
        ];

        for (ancestor, descendant, expected) in cases {
            assert_eq!(
                resolve_chain_effective_privacy([ancestor, descendant]),
                expected,
                "Failed resolving pair: ({ancestor:?}, {descendant:?})"
            );
        }
    }

    #[test]
    fn multi_level_n_depth_privacy_inheritance_propagation() {
        use super::resolve_chain_effective_privacy;

        // Level 1: Local Only -> Level 2: None/Open -> Level 3: Open => Local Only
        assert_eq!(
            resolve_chain_effective_privacy([Some(TIER_LOCAL_ONLY), None, Some(TIER_OPEN)]),
            TIER_LOCAL_ONLY
        );

        // Level 1: Open -> Level 2: None -> Level 3: Redacted => Redacted
        assert_eq!(
            resolve_chain_effective_privacy([Some(TIER_OPEN), None, Some(TIER_REDACTED)]),
            TIER_REDACTED
        );

        // Level 1: Open -> Level 2: Open -> Level 3: Redacted -> Level 4: Open -> Level 5: Open => Redacted
        let deep_chain = [
            Some(TIER_OPEN),
            Some(TIER_OPEN),
            Some(TIER_REDACTED),
            Some(TIER_OPEN),
            Some(TIER_OPEN),
        ];
        assert_eq!(resolve_chain_effective_privacy(deep_chain), TIER_REDACTED);
    }
}
