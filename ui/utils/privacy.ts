const PRIVACY_RANKS: Record<string, number> = {
  open: 0,
  local_only: 1,
  locked: 2,
  redacted: 3,
};

type VaultHierarchyLike = {
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
  const name = (vault as { name?: string }).name ?? "Vault";
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
    const name = (vault as { name?: string }).name ?? "Vault";
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
