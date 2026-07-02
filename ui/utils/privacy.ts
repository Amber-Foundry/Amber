/**
 * Amber privacy tiers — four tiers, two axes. Rust source of truth: core/src/privacy.rs
 *
 * Axis 1 (egress): open → cloud+local | local_only → local only | locked → cloud stub |
 *   redacted → omitted from cloud
 * Axis 2 (disclosure): open/local → full UI | locked → title visible, body gated |
 *   redacted → metadata hidden + encrypted at rest
 */
const PRIVACY_RANKS: Record<string, number> = {
  open: 0,
  local_only: 1,
  locked: 2,
  redacted: 3,
};

type VaultHierarchyLike = {
  name?: string;
  parentVaultId?: string | null;
  privacyTier?: string | null;
};

function normalizeTier(tier?: string | null): string {
  if (!tier) {
    return "open";
  }
  return tier in PRIVACY_RANKS ? tier : "open";
}

export const getPrivacyRank = (tier?: string | null): number => PRIVACY_RANKS[normalizeTier(tier)];

export function isRedactedLocked(tier?: string | null, isUnlocked = false): boolean {
  return normalizeTier(tier) === "redacted" && !isUnlocked;
}

export function shouldOmitSpatialConnector(
  effectiveTier: string,
  isRedactedUnlocked: boolean
): boolean {
  return isRedactedLocked(effectiveTier, isRedactedUnlocked);
}

export function getPrivacyDisplayLabel(
  label: string,
  tier?: string | null,
  isUnlocked = false,
  redactedLabel = "[REDACTED]"
): string {
  return isRedactedLocked(tier, isUnlocked) ? redactedLabel : label;
}

export function getPrivacyDisplaySummary(
  summary: string | null | undefined,
  tier?: string | null,
  isUnlocked = false,
  redactedSummary = "[Metadata Locked]"
): string {
  if (isRedactedLocked(tier, isUnlocked)) {
    return redactedSummary;
  }
  return (summary || "").trim();
}

export function getVaultDisplayLabel(
  vaultId: string,
  vaultsById: Record<string, VaultHierarchyLike | undefined>,
  isUnlocked = false,
  redactedLabel = "[REDACTED]"
): string {
  const vault = vaultsById[vaultId];
  if (!vault) {
    return "Unknown Vault";
  }
  const effectiveTier = getVaultEffectivePrivacy(vaultId, vaultsById);
  const name = vault.name ?? "Vault";
  return getPrivacyDisplayLabel(name, effectiveTier, isUnlocked, redactedLabel);
}

export function getVaultDisplayPath(
  vaultId: string,
  vaultsById: Record<string, VaultHierarchyLike | undefined>,
  isUnlocked = false
): string {
  const segments: string[] = [];
  const seen = new Set<string>();
  let currentId: string | null | undefined = vaultId;

  while (currentId) {
    if (seen.has(currentId)) {
      break;
    }
    seen.add(currentId);
    const vault: VaultHierarchyLike | undefined = vaultsById[currentId];
    if (!vault) {
      break;
    }

    const effectiveTier = getVaultEffectivePrivacy(currentId, vaultsById);
    const name = vault.name ?? "Vault";
    segments.push(getPrivacyDisplayLabel(name, effectiveTier, isUnlocked));
    currentId = vault.parentVaultId ?? null;
  }

  if (segments.length === 0) {
    return "Unknown Vault";
  }

  return segments.reverse().join(" / ");
}

export function getEffectivePrivacy(
  nodeTier?: string | null,
  subVaultTier?: string | null,
  vaultTier?: string | null
): string {
  const tiers = [normalizeTier(nodeTier), normalizeTier(subVaultTier), normalizeTier(vaultTier)];

  return tiers.reduce((strictest, current) =>
    getPrivacyRank(current) > getPrivacyRank(strictest) ? current : strictest
  );
}

export function getVaultEffectivePrivacy(
  vaultId: string,
  vaultsById: Record<string, VaultHierarchyLike | undefined>,
  memo: Record<string, string> = {},
  seen: Set<string> = new Set()
): string {
  if (memo[vaultId]) {
    return memo[vaultId];
  }

  if (seen.has(vaultId)) {
    return "open";
  }

  const vault = vaultsById[vaultId];
  if (!vault) {
    return "open";
  }

  seen.add(vaultId);
  const parentTier = vault.parentVaultId
    ? getVaultEffectivePrivacy(vault.parentVaultId, vaultsById, memo, seen)
    : null;
  seen.delete(vaultId);

  const effectiveTier = getEffectivePrivacy(vault.privacyTier, parentTier);
  memo[vaultId] = effectiveTier;
  return effectiveTier;
}

/**
 * Self-test suite to verify privacy hierarchy calculations and cycle prevention.
 */
export function runPrivacyTests() {
  const mockVaults: Record<string, VaultHierarchyLike> = {
    vault_open: { name: "Open Vault", privacyTier: "open" },
    vault_local: { name: "Local Vault", privacyTier: "local_only" },
    vault_locked: { name: "Locked Vault", privacyTier: "locked", parentVaultId: "vault_local" },
    vault_redacted: { name: "Redacted Vault", privacyTier: "redacted" },
  };

  // Test 1: Simple hierarchy
  const tier = getVaultEffectivePrivacy("vault_locked", mockVaults);
  if (tier !== "locked") {
    throw new Error(`Privacy Test 1 Failed: Expected locked, got ${tier}`);
  }

  // Test 2: Cycle detection (Self-referencing cycle A -> B -> A)
  const cyclicVaults: Record<string, VaultHierarchyLike> = {
    vault_a: { name: "Vault A", privacyTier: "open", parentVaultId: "vault_b" },
    vault_b: { name: "Vault B", privacyTier: "locked", parentVaultId: "vault_a" },
  };

  // This should not infinite loop and should resolve vault_a to locked (from vault_b)
  const tierA = getVaultEffectivePrivacy("vault_a", cyclicVaults);
  const tierB = getVaultEffectivePrivacy("vault_b", cyclicVaults);
  if (tierA !== "locked") {
    throw new Error(`Privacy Test 2 Failed: Expected locked for A, got ${tierA}`);
  }
  if (tierB !== "locked") {
    throw new Error(`Privacy Test 2 Failed: Expected locked for B, got ${tierB}`);
  }

  // Test 3: Self-referencing (A -> A)
  const selfRefVaults: Record<string, VaultHierarchyLike> = {
    vault_self: { name: "Self Vault", privacyTier: "local_only", parentVaultId: "vault_self" },
  };
  const tierSelf = getVaultEffectivePrivacy("vault_self", selfRefVaults);
  if (tierSelf !== "local_only") {
    throw new Error(`Privacy Test 3 Failed: Expected local_only, got ${tierSelf}`);
  }

  // Test 4: getVaultDisplayPath - Standard hierarchy
  const displayPath1 = getVaultDisplayPath("vault_locked", mockVaults);
  if (displayPath1 !== "Local Vault / Locked Vault") {
    throw new Error(
      `Privacy Test 4 Failed: Expected 'Local Vault / Locked Vault', got '${displayPath1}'`
    );
  }

  // Test 5: getVaultDisplayPath - Redacted cascade (locked vs unlocked)
  const mockRedacted: Record<string, VaultHierarchyLike> = {
    vault_p: { name: "Secret", privacyTier: "redacted" },
    vault_c: { name: "Plan", privacyTier: "open", parentVaultId: "vault_p" },
  };
  const displayPathLocked = getVaultDisplayPath("vault_c", mockRedacted, false);
  const displayPathUnlocked = getVaultDisplayPath("vault_c", mockRedacted, true);
  if (displayPathLocked !== "[REDACTED] / [REDACTED]") {
    throw new Error(
      `Privacy Test 5a Failed: Expected '[REDACTED] / [REDACTED]', got '${displayPathLocked}'`
    );
  }
  if (displayPathUnlocked !== "Secret / Plan") {
    throw new Error(
      `Privacy Test 5b Failed: Expected 'Secret / Plan', got '${displayPathUnlocked}'`
    );
  }

  // Test 6: getVaultDisplayPath - Cycle breaking
  const displayPathCycle = getVaultDisplayPath("vault_b", cyclicVaults);
  if (displayPathCycle !== "Vault A / Vault B") {
    throw new Error(
      `Privacy Test 6 Failed: Expected 'Vault A / Vault B', got '${displayPathCycle}'`
    );
  }

  // Test 7: getVaultDisplayPath - Nonexistent vault
  const displayPathNonexistent = getVaultDisplayPath("nonexistent", mockVaults);
  if (displayPathNonexistent !== "Unknown Vault") {
    throw new Error(
      `Privacy Test 7 Failed: Expected 'Unknown Vault', got '${displayPathNonexistent}'`
    );
  }

  // Test 8: getPrivacyDisplaySummary - Nullish and empty cases when NOT redacted
  const s1 = getPrivacyDisplaySummary("  My Summary  ", "open", false);
  if (s1 !== "My Summary") {
    throw new Error(`Privacy Test 8a Failed: Expected 'My Summary', got '${s1}'`);
  }
  const s2 = getPrivacyDisplaySummary(null, "open", false);
  if (s2 !== "") {
    throw new Error(`Privacy Test 8b Failed: Expected '', got '${s2}'`);
  }
  const s3 = getPrivacyDisplaySummary(undefined, "open", false);
  if (s3 !== "") {
    throw new Error(`Privacy Test 8c Failed: Expected '', got '${s3}'`);
  }
  const s4 = getPrivacyDisplaySummary("   ", "open", false);
  if (s4 !== "") {
    throw new Error(`Privacy Test 8d Failed: Expected '', got '${s4}'`);
  }

  // Test 9: getPrivacyDisplaySummary - Privacy tiers with isUnlocked = false
  const s5 = getPrivacyDisplaySummary("Secret info", "redacted", false);
  if (s5 !== "[Metadata Locked]") {
    throw new Error(`Privacy Test 9a Failed: Expected '[Metadata Locked]', got '${s5}'`);
  }
  const s6 = getPrivacyDisplaySummary("Secret info", "redacted", false, "Custom Redacted");
  if (s6 !== "Custom Redacted") {
    throw new Error(`Privacy Test 9b Failed: Expected 'Custom Redacted', got '${s6}'`);
  }
  const s7 = getPrivacyDisplaySummary("Secret info", "locked", false);
  if (s7 !== "Secret info") {
    throw new Error(`Privacy Test 9c Failed: Expected 'Secret info', got '${s7}'`);
  }
  const s8 = getPrivacyDisplaySummary("Secret info", "local_only", false);
  if (s8 !== "Secret info") {
    throw new Error(`Privacy Test 9d Failed: Expected 'Secret info', got '${s8}'`);
  }

  // Test 10: getPrivacyDisplaySummary - Privacy tiers with isUnlocked = true
  const s9 = getPrivacyDisplaySummary("Secret info", "redacted", true);
  if (s9 !== "Secret info") {
    throw new Error(`Privacy Test 10 Failed: Expected 'Secret info', got '${s9}'`);
  }
}
