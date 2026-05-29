import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import process from "node:process";

const args = new Set(process.argv.slice(2));
const fixFromArgs = args.has("--fix");
const fixFromNpmArgv = (() => {
  // Some npm environments (notably on Windows) may not forward `-- <args>`
  // to the underlying command consistently. As a fallback, inspect npm's argv.
  const raw = process.env.npm_config_argv;
  if (!raw) {
    return false;
  }
  try {
    const parsed = JSON.parse(raw);
    return Boolean(
      parsed &&
      parsed.original &&
      Array.isArray(parsed.original) &&
      parsed.original.includes("--fix")
    );
  } catch {
    return raw.includes("--fix");
  }
})();
const fix = fixFromArgs || fixFromNpmArgv;
const help = args.has("--help") || args.has("-h");
const MIN_NODE_VERSION = [22, 6, 0];

function compareVersions(actual, minimum) {
  for (let index = 0; index < minimum.length; index++) {
    const actualPart = actual[index] ?? 0;
    const minimumPart = minimum[index] ?? 0;
    if (actualPart > minimumPart) return 1;
    if (actualPart < minimumPart) return -1;
  }
  return 0;
}

function assertNodeVersion() {
  const actual = process.versions.node.split(".").map((part) => Number(part));
  if (compareVersions(actual, MIN_NODE_VERSION) < 0) {
    const required = MIN_NODE_VERSION.join(".");
    const detected = process.versions.node;
    console.error(
      `Node.js ${required}+ is required for preflight because it uses --experimental-strip-types. ` +
        `Detected ${detected}. Please upgrade Node.js or run the individual checks manually.`
    );
    process.exit(1);
  }
}

if (help) {
  // Keep this intentionally short and copy-paste friendly.
  console.log(`MindVault preflight checks

Usage:
  npm run preflight
  npm run preflight -- --fix

What it runs:
  - Prettier + ESLint + TypeScript (UI)
  - cargo fmt/clippy/test (core)
`);
  process.exit(0);
}

assertNodeVersion();

function run(command, { cwd } = {}) {
  return new Promise((resolve) => {
    const child = spawn(command, {
      cwd,
      stdio: "inherit",
      shell: true,
      env: process.env,
    });
    child.on("exit", (code) => resolve(code ?? 1));
    child.on("error", () => resolve(1));
  });
}

function getBundledRipgrepPath() {
  try {
    const require = createRequire(import.meta.url);
    const rgPath = require("@vscode/ripgrep").rgPath;
    if (typeof rgPath === "string" && rgPath.length > 0) {
      return rgPath.includes(" ") ? `"${rgPath}"` : rgPath;
    }
  } catch {
    // VSCode ripgrep package is not installed or resolution failed.
  }
  return null;
}

function getPathRipgrepCommand() {
  // Fall back to the globally installed `rg` binary on system PATH.
  return "rg";
}

function getRgCommand() {
  // Prefer the bundled cross-platform local ripgrep binary, falling back to system PATH.
  return getBundledRipgrepPath() ?? getPathRipgrepCommand();
}

async function assertRgNoMatches({ name, args }) {
  // ripgrep exit codes:
  // 0 = matches found
  // 1 = no matches
  // 2 = error
  const cmd = args.join(" ");
  const code = await run(cmd);
  if (code === 0) {
    console.error(`\nBanned pattern matched: ${name}`);
    return 1;
  }
  if (code === 1) {
    return 0;
  }
  console.error(`\nBanned pattern check errored: ${name}`);
  return code;
}

// Banned pattern regex to detect sensitive credentials printed in Rust logging statements.
const BANNED_LOGGING_CREDENTIALS_REGEX =
  '"(tracing|log)::(trace|debug|info|warn|error)!\\([^\\n]*(api_key|password|secret|token)\\s*="';

async function runBannedPatterns() {
  const rg = getRgCommand();
  const checks = [
    {
      name: "XSS: dangerouslySetInnerHTML in ui/",
      args: [rg, '"dangerouslySetInnerHTML"', "ui", "--glob", '"*.ts"', "--glob", '"*.tsx"'],
    },
    {
      name: "IPC: invoke() directly in ui/components/",
      args: [rg, '"invoke\\("', "ui/components"],
    },
    {
      name: "TypeScript: explicit any in ui/",
      args: [rg, '": any\\b|as any\\b"', "ui", "--glob", '"*.ts"', "--glob", '"*.tsx"'],
    },
    {
      name: "Rust logging: secret-ish fields in core/src/",
      args: [rg, BANNED_LOGGING_CREDENTIALS_REGEX, "core/src"],
    },
  ];

  for (const check of checks) {
    const code = await assertRgNoMatches(check);
    if (code !== 0) {
      return code;
    }
  }
  return 0;
}

const CARGO_MANIFEST_FLAGS = ["--manifest-path", "core/Cargo.toml"];

const CARGO_FMT_CMD = fix
  ? ["cargo", "fmt", ...CARGO_MANIFEST_FLAGS]
  : ["cargo", "fmt", ...CARGO_MANIFEST_FLAGS, "--", "--check"];

const CARGO_CLIPPY_CMD = [
  "cargo",
  "clippy",
  ...CARGO_MANIFEST_FLAGS,
  "--all-targets",
  "--",
  "-D",
  "warnings",
  "-D",
  "clippy::unwrap_used",
  "-D",
  "clippy::expect_used",
];

const CARGO_TEST_CMD = ["cargo", "test", ...CARGO_MANIFEST_FLAGS];

const steps = [
  {
    name: fix ? "prettier (write)" : "prettier (check)",
    cmd: fix ? "npx prettier --write ." : "npx prettier --check .",
  },
  {
    name: fix ? "eslint (fix)" : "eslint",
    cmd: fix ? "npx eslint . --fix" : "npx eslint .",
  },
  { name: "banned patterns", cmd: runBannedPatterns },
  { name: "tsc (noEmit)", cmd: "npx tsc --noEmit" },
  {
    name: "frontend utility tests",
    cmd: "node --experimental-strip-types scripts/test-frontend.ts",
  },
  {
    name: fix ? "cargo fmt" : "cargo fmt (check)",
    cmd: CARGO_FMT_CMD.join(" "),
  },
  {
    name: "cargo clippy",
    cmd: CARGO_CLIPPY_CMD.join(" "),
  },
  { name: "cargo test", cmd: CARGO_TEST_CMD.join(" ") },
  {
    name: fix ? "format generated types (write)" : "format generated types (check)",
    cmd: fix
      ? "npx prettier --write ui/types/generated"
      : "npx prettier --check ui/types/generated",
  },
];

for (const step of steps) {
  console.log(`\n==> ${step.name}`);
  const code = typeof step.cmd === "function" ? await step.cmd() : await run(step.cmd);
  if (code !== 0) {
    console.error(`\nPreflight failed: ${step.name}`);
    process.exit(code);
  }
}

console.log("\nPreflight passed.");
