import React from "react";

/**
 * Vault icons are persisted in `Vault.icon` as either a legacy emoji glyph or a
 * stable kebab-case key. We render them as MIT-licensed Lucide SVGs (the same
 * icon set already used across the app) so no emoji glyphs are ever rendered.
 *
 * Legacy emoji stored in the DB are mapped to their key via LEGACY_EMOJI_TO_KEY
 * so existing vaults keep their chosen icon without a migration.
 */

export const VAULT_ICON_KEYS = [
  "credit-card",
  "coins",
  "fitness",
  "book",
  "user",
  "briefcase",
  "home",
  "mobile",
  "laptop",
  "note",
  "brain",
  "piggy",
  "key",
  "palette",
  "rocket",
  "folder",
] as const;

export type VaultIconKey = (typeof VAULT_ICON_KEYS)[number];

// SVG path data for each key (Lucide, MIT licensed). viewBox 0 0 24 24.
const VAULT_ICON_PATHS: Record<VaultIconKey, React.ReactNode> = {
  "credit-card": (
    <>
      <rect width="20" height="14" x="2" y="5" rx="2" />
      <line x1="2" x2="22" y1="10" y2="10" />
    </>
  ),
  coins: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M18.09 10.37A6 6 0 1 1 10.34 18" />
      <path d="M7 6h1v4" />
      <path d="m16.71 19.362 1.42-1.42" />
    </>
  ),
  fitness: (
    <>
      <path d="M14.4 14.4 9.6 9.6" />
      <path d="M18.657 21.485a2 2 0 1 1-2.829-2.828l-1.767 1.768a2 2 0 1 1-2.829-2.829l6.364-6.364a2 2 0 1 1 2.829 2.829l-1.768 1.767a2 2 0 1 1 2.828 2.829z" />
      <path d="m21.5 21.5-1.4-1.4" />
      <path d="M3.9 3.9 2.5 2.5" />
      <path d="M6.404 12.768a2 2 0 1 1-2.829-2.829l1.768-1.767a2 2 0 1 1-2.829-2.828l2.828-2.828a2 2 0 1 1 2.829 2.829l1.767-1.768a2 2 0 1 1 2.829 2.829z" />
    </>
  ),
  book: (
    <>
      <path d="M12 7v14" />
      <path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z" />
    </>
  ),
  user: (
    <>
      <path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2" />
      <circle cx="12" cy="7" r="4" />
    </>
  ),
  briefcase: (
    <>
      <rect width="20" height="14" x="2" y="7" rx="2" />
      <path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16" />
    </>
  ),
  home: (
    <>
      <path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <polyline points="9 22 9 12 15 12 15 22" />
    </>
  ),
  mobile: (
    <>
      <rect width="14" height="20" x="5" y="2" rx="2" />
      <path d="M12 18h.01" />
    </>
  ),
  laptop: (
    <>
      <path d="M20 16V7a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v9" />
      <path d="M2 20h20" />
    </>
  ),
  note: (
    <>
      <path d="M14 3v4a1 1 0 0 0 1 1h4" />
      <path d="M17 21H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7l5 5v11a2 2 0 0 1-2 2z" />
      <path d="M9 13h6" />
      <path d="M9 17h3" />
    </>
  ),
  brain: (
    <>
      <path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z" />
      <path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z" />
      <path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4" />
      <path d="M17.599 6.5a3 3 0 0 0 .399-1.375" />
      <path d="M6.003 5.125A3 3 0 0 0 6.401 6.5" />
      <path d="M3.477 10.896a4 4 0 0 1 .585-.396" />
      <path d="M19.938 10.5a4 4 0 0 1 .585.396" />
      <path d="M6 18a4 4 0 0 1-1.967-.516" />
      <path d="M19.967 17.484A4 4 0 0 1 18 18" />
    </>
  ),
  piggy: (
    <>
      <path d="M19 14c1.49 0 3 1.343 3 3v0a1.5 1.5 0 0 1-3 0" />
      <path d="M10.6 4.9A2 2 0 0 1 12 6.5v9a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h.6a2 2 0 0 1 1.4-.6Z" />
      <path d="M16 11h .01" />
      <circle cx="16" cy="11" r=".5" />
      <path d="M13 21v-2a2 2 0 0 1 2-2h1.8a4 4 0 0 0 3.2-1.6l.9-.9" />
    </>
  ),
  key: (
    <>
      <path d="m15.5 7.5 2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L19 4" />
      <path d="m21 2-9.6 9.6" />
      <circle cx="7.5" cy="15.5" r="5.5" />
    </>
  ),
  palette: (
    <>
      <circle cx="13.5" cy="6.5" r=".5" fill="currentColor" />
      <circle cx="17.5" cy="10.5" r=".5" fill="currentColor" />
      <circle cx="8.5" cy="7.5" r=".5" fill="currentColor" />
      <circle cx="6.5" cy="12.5" r=".5" fill="currentColor" />
      <path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.555C21.965 6.012 17.461 2 12 2Z" />
    </>
  ),
  rocket: (
    <>
      <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z" />
      <path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z" />
      <path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0" />
      <path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5" />
    </>
  ),
  folder: (
    <>
      <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
    </>
  ),
};

const LEGACY_EMOJI_TO_KEY: Record<string, VaultIconKey> = {
  "💳": "credit-card",
  "🪙": "coins",
  "💪": "fitness",
  "📚": "book",
  "👤": "user",
  "💼": "briefcase",
  "🏠": "home",
  "📱": "mobile",
  "💻": "laptop",
  "📝": "note",
  "🧠": "brain",
  "💰": "piggy",
  "🔑": "key",
  "🎨": "palette",
  "🚀": "rocket",
  "📂": "folder",
};

// Keyword heuristics previously stored as `Vault.icon` (e.g. seeded vaults).
const KEYWORD_TO_KEY: Record<string, VaultIconKey> = {
  key: "credit-card",
  credentials: "credit-card",
  coins: "coins",
  finance: "coins",
  money: "coins",
  heart: "fitness",
  health: "fitness",
  fitness: "fitness",
  book: "book",
  learning: "book",
  read: "book",
  user: "user",
  personal: "user",
  briefcase: "briefcase",
  work: "briefcase",
  project: "briefcase",
  home: "home",
  mobile: "mobile",
  phone: "mobile",
  cse: "mobile",
  classes: "laptop",
  computer: "laptop",
  laptop: "laptop",
  note: "note",
  brain: "brain",
  piggy: "piggy",
  piggybank: "piggy",
  palette: "palette",
  rocket: "rocket",
  folder: "folder",
};

// Name-based fallback used when no explicit icon is stored.
function resolveFromName(name: string): VaultIconKey {
  const n = name.toLowerCase();
  if (n.includes("home") || n.includes("vault 1")) return "home";
  if (n.includes("class") || n.includes("cse")) return "laptop";
  if (n.includes("credential") || n.includes("password") || n.includes("key")) return "credit-card";
  if (n.includes("secret")) return "key";
  if (n.includes("personal") || n.includes("self")) return "user";
  if (n.includes("book") || n.includes("learn") || n.includes("study")) return "book";
  if (n.includes("work") || n.includes("job") || n.includes("project") || n.includes("brief"))
    return "briefcase";
  if (n.includes("finance") || n.includes("money") || n.includes("bank") || n.includes("coin"))
    return "coins";
  if (n.includes("health") || n.includes("fit")) return "fitness";
  if (n.includes("phone") || n.includes("mobile")) return "mobile";
  if (n.includes("computer") || n.includes("laptop")) return "laptop";
  if (n.includes("note") || n.includes("idea")) return "note";
  if (n.includes("brain") || n.includes("memory")) return "brain";
  if (n.includes("art") || n.includes("design")) return "palette";
  if (n.includes("launch") || n.includes("ship") || n.includes("rocket")) return "rocket";
  return "folder";
}

/**
 * Resolves a stored `Vault.icon` value (legacy emoji, keyword, or kebab key)
 * into a renderable key. Falls back to a name heuristic when empty/unknown.
 */
export function resolveVaultIconKey(
  icon: string | null | undefined,
  name: string = ""
): VaultIconKey {
  const raw = (icon || "").trim();
  const lower = raw.toLowerCase();

  if (LEGACY_EMOJI_TO_KEY[raw]) return LEGACY_EMOJI_TO_KEY[raw];
  if (lower && lower in KEYWORD_TO_KEY) return KEYWORD_TO_KEY[lower];
  if (VAULT_ICON_KEYS.includes(lower as VaultIconKey)) {
    return lower as VaultIconKey;
  }
  return resolveFromName(name);
}

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
