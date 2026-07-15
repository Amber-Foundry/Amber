import { GlobeIcon, HomeIcon, LockIcon, SquareIcon } from "./icons";

const TIER_ICONS: Record<string, React.ReactNode> = {
  open: <GlobeIcon size={14} />,
  local_only: <HomeIcon size={14} />,
  locked: <LockIcon size={14} />,
  redacted: <SquareIcon size={14} />,
};

export function PrivacyBadge({ tier, className }: { tier: string; className?: string }) {
  const normalizedTier = tier in TIER_ICONS ? tier : "open";
  const icon = TIER_ICONS[normalizedTier];

  return <span className={`privacy-badge ${normalizedTier} ${className || ""}`}>{icon}</span>;
}
