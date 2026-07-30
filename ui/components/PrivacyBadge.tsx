import { GlobeIcon, HomeIcon, SquareIcon } from "./icons";

const TIER_ICONS: Record<string, React.ReactNode> = {
  open: <GlobeIcon size={14} />,
  local_only: <HomeIcon size={14} />,
  redacted: <SquareIcon size={14} />,
};

function normalizeBadgeTier(tier: string): string {
  if (tier === "locked") return "redacted";
  return tier in TIER_ICONS ? tier : "open";
}

export function PrivacyBadge({ tier, className }: { tier: string; className?: string }) {
  const normalizedTier = normalizeBadgeTier(tier);
  const icon = TIER_ICONS[normalizedTier];

  return <span className={`privacy-badge ${normalizedTier} ${className || ""}`}>{icon}</span>;
}
