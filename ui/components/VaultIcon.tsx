import { resolveVaultIconKey, VAULT_ICON_PATHS } from "./VaultIconUtils";

export function VaultIcon({
  icon,
  name,
  size = 18,
  className,
}: {
  icon?: string | null;
  name?: string;
  size?: number;
  className?: string;
}) {
  const key = resolveVaultIconKey(icon, name);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {VAULT_ICON_PATHS[key]}
    </svg>
  );
}

export default VaultIcon;
