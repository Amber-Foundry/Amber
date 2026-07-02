//! Amber privacy tiers — four tiers, two independent axes.
//!
//! ## Axis 1: Egress (where may AI see this?)
//!
//! | Tier        | Cloud LLM | Local LLM (device)      |
//! |-------------|-----------|-------------------------|
//! | `open`      | full      | full                    |
//! | `local_only`| omit      | full                    |
//! | `locked`    | stub      | full when session unlocked; stub otherwise |
//! | `redacted`  | omit      | omit (assembler never includes encrypted nodes) |
//!
//! ## Axis 2: Disclosure (what is visible before unlock?)
//!
//! | Tier        | UI metadata (title, nav) | Body content | At-rest encryption |
//! |-------------|----------------------------|--------------|--------------------|
//! | `open`      | visible                    | visible      | no                 |
//! | `local_only`| visible                    | visible      | no                 |
//! | `locked`    | visible                    | gated        | no                 |
//! | `redacted`  | hidden (`[REDACTED]`)      | gated        | yes                |
//!
//! ## Durable indexes (embeddings, exports)
//!
//! Persisted vectors must not store cleartext that the tier withholds from cloud context.
//! Session unlock gates UI reading and *ephemeral* local LLM context — not promotion of
//! locked content into durable indexes.
//!
//! | Tier        | Embedding policy                          |
//! |-------------|-------------------------------------------|
//! | `open`      | full cleartext                            |
//! | `local_only`| full cleartext (local ONNX / local Ollama)|
//! | `locked`    | pointer stub always                       |
//! | `redacted`  | skip (delete existing vectors)            |
//!
//! Effective tier resolves as the strictest of node, sub-vault, and vault tiers.
//! All subsystems (`llm::assembler`, `embed::job`, UI helpers) should follow this matrix.

pub const TIER_OPEN: &str = "open";
pub const TIER_LOCAL_ONLY: &str = "local_only";
pub const TIER_LOCKED: &str = "locked";
pub const TIER_REDACTED: &str = "redacted";

const OPEN: &str = TIER_OPEN;
const LOCAL_ONLY: &str = TIER_LOCAL_ONLY;
const LOCKED: &str = TIER_LOCKED;
const REDACTED: &str = TIER_REDACTED;

/// Whether full node content may be sent to a cloud LLM.
pub fn allows_cloud_content(tier: &str) -> bool {
    normalize_tier(Some(tier)) == OPEN
}

/// Whether a pointer stub (title + id) may be sent to a cloud LLM.
pub fn allows_cloud_stub(tier: &str) -> bool {
    normalize_tier(Some(tier)) == LOCKED
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

/// Whether embeddings must use a pointer stub instead of cleartext chunks.
pub fn embedding_uses_stub(tier: &str) -> bool {
    normalize_tier(Some(tier)) == LOCKED
}

/// Local LLM context for locked tier: stub unless the redacted session is unlocked.
pub fn local_llm_locked_uses_full_content_when_unlocked(is_unlocked: bool) -> bool {
    is_unlocked
}

/// Whether a node must be omitted from local LLM context assembly.
pub fn omits_from_local_llm(tier: &str) -> bool {
    normalize_tier(Some(tier)) == REDACTED
}

/// Whether local-only nodes must not embed via a non-loopback Ollama endpoint.
pub fn embedding_blocks_on_remote_ollama(tier: &str) -> bool {
    normalize_tier(Some(tier)) == LOCAL_ONLY
}

/// How a node should appear in assembled LLM context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmContextPolicy {
    Full,
    Stub,
    Omit,
}

pub fn cloud_llm_context_policy(tier: &str) -> LlmContextPolicy {
    if allows_cloud_content(tier) {
        LlmContextPolicy::Full
    } else if allows_cloud_stub(tier) {
        LlmContextPolicy::Stub
    } else {
        LlmContextPolicy::Omit
    }
}

pub fn local_llm_context_policy(tier: &str, is_unlocked: bool) -> LlmContextPolicy {
    if omits_from_local_llm(tier) {
        LlmContextPolicy::Omit
    } else if allows_cloud_stub(tier)
        && !local_llm_locked_uses_full_content_when_unlocked(is_unlocked)
    {
        LlmContextPolicy::Stub
    } else {
        LlmContextPolicy::Full
    }
}

/// Unrestricted/debug scopes: full content for non-redacted tiers.
pub fn unrestricted_llm_context_policy(tier: &str) -> LlmContextPolicy {
    if omits_from_local_llm(tier) {
        LlmContextPolicy::Omit
    } else {
        LlmContextPolicy::Full
    }
}

fn normalize_tier(tier: Option<&str>) -> &'static str {
    match tier {
        Some(LOCAL_ONLY) => LOCAL_ONLY,
        Some(LOCKED) => LOCKED,
        Some(REDACTED) => REDACTED,
        _ => OPEN,
    }
}

pub fn get_privacy_rank(tier: Option<&str>) -> u8 {
    match normalize_tier(tier) {
        OPEN => 0,
        LOCAL_ONLY => 1,
        LOCKED => 2,
        REDACTED => 3,
        _ => 0,
    }
}

pub fn get_effective_privacy(
    node_tier: Option<&str>,
    sub_vault_tier: Option<&str>,
    vault_tier: Option<&str>,
) -> &'static str {
    let tiers = [
        normalize_tier(node_tier),
        normalize_tier(sub_vault_tier),
        normalize_tier(vault_tier),
    ];

    let mut strictest = OPEN;
    for tier in tiers {
        if get_privacy_rank(Some(tier)) > get_privacy_rank(Some(strictest)) {
            strictest = tier;
        }
    }
    strictest
}

pub fn generate_pointer_stub(node_title: &str, node_id: &str) -> String {
    format!(
        "[LOCKED NODE STUB] Title: {} (ID: {}) - Content withheld due to privacy constraints.",
        node_title, node_id
    )
}

#[cfg(test)]
mod tests {
    use super::{
        allows_cloud_content, allows_cloud_stub, cloud_llm_context_policy,
        embedding_blocks_on_remote_ollama, embedding_should_skip, embedding_uses_stub,
        encrypts_at_rest, get_effective_privacy, get_privacy_rank, hides_metadata_until_unlock,
        local_llm_context_policy, local_llm_locked_uses_full_content_when_unlocked,
        omits_from_local_llm, unrestricted_llm_context_policy, LlmContextPolicy, TIER_LOCAL_ONLY,
        TIER_LOCKED, TIER_OPEN, TIER_REDACTED,
    };

    #[test]
    fn privacy_waterfall_parent_locked_beats_node_open() {
        let effective = get_effective_privacy(Some("open"), Some("locked"), None);
        assert_eq!(effective, "locked");
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
    fn privacy_waterfall_redacted_beats_locked_across_hierarchy() {
        let effective = get_effective_privacy(Some("open"), Some("locked"), Some("redacted"));
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
        assert!(!allows_cloud_content(TIER_LOCKED));
        assert!(allows_cloud_stub(TIER_LOCKED));
        assert!(!allows_cloud_stub(TIER_OPEN));
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
        assert!(!encrypts_at_rest(TIER_LOCKED));
        assert!(hides_metadata_until_unlock(TIER_REDACTED));
        assert!(!hides_metadata_until_unlock(TIER_LOCKED));
        assert!(embedding_should_skip(TIER_REDACTED));
        assert!(!embedding_should_skip(TIER_LOCKED));
        assert!(embedding_uses_stub(TIER_LOCKED));
        assert!(!embedding_uses_stub(TIER_OPEN));
    }

    #[test]
    fn locked_local_llm_respects_session_unlock() {
        assert!(!local_llm_locked_uses_full_content_when_unlocked(false));
        assert!(local_llm_locked_uses_full_content_when_unlocked(true));
    }

    #[test]
    fn llm_context_policy_matrix() {
        assert_eq!(cloud_llm_context_policy(TIER_OPEN), LlmContextPolicy::Full);
        assert_eq!(
            cloud_llm_context_policy(TIER_LOCKED),
            LlmContextPolicy::Stub
        );
        assert_eq!(
            cloud_llm_context_policy(TIER_LOCAL_ONLY),
            LlmContextPolicy::Omit
        );
        assert_eq!(
            local_llm_context_policy(TIER_LOCKED, false),
            LlmContextPolicy::Stub
        );
        assert_eq!(
            local_llm_context_policy(TIER_LOCKED, true),
            LlmContextPolicy::Full
        );
        assert_eq!(
            local_llm_context_policy(TIER_REDACTED, true),
            LlmContextPolicy::Omit
        );
        assert_eq!(
            unrestricted_llm_context_policy(TIER_LOCKED),
            LlmContextPolicy::Full
        );
        assert!(omits_from_local_llm(TIER_REDACTED));
        assert!(embedding_blocks_on_remote_ollama(TIER_LOCAL_ONLY));
    }
}
