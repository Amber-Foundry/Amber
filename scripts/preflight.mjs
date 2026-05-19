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

function getRgCommand() {
  // Prefer a local ripgrep binary (cross-platform) when available.
  // Falls back to `rg` on PATH if not installed.
  try {
    const require = createRequire(import.meta.url);
    const rgPath = require("@vscode/ripgrep").rgPath;
    if (typeof rgPath === "string" && rgPath.length > 0) {
      const quoted = rgPath.includes(" ") ? `"${rgPath}"` : rgPath;
      return quoted;
    }
  } catch {
    // ignore
  }
  return "rg";
}

async function assertRgNoMatches({ name, cmd }) {
  // ripgrep exit codes:
  // 0 = matches found
  // 1 = no matches
  // 2 = error
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

async function runBannedPatterns() {
  const rg = getRgCommand();
  const checks = [
    {
      name: "XSS: dangerouslySetInnerHTML in ui/",
      cmd: `${rg} "dangerouslySetInnerHTML" ui --glob "*.ts" --glob "*.tsx"`,
    },
    {
      name: "IPC: invoke() directly in ui/components/",
      cmd: `${rg} "invoke\\(" ui/components`,
    },
    {
      name: "TypeScript: explicit any in ui/",
      cmd: `${rg} ": any\\b|as any\\b" ui --glob "*.ts" --glob "*.tsx"`,
    },
    {
      name: "Rust logging: secret-ish fields in core/src/",
      cmd: `${rg} "(tracing|log)::(trace|debug|info|warn|error)!\\([^\\n]*(api_key|password|secret|token)\\s*=" core/src`,
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
    name: fix ? "cargo fmt" : "cargo fmt (check)",
    cmd: fix
      ? "cargo fmt --manifest-path core/Cargo.toml"
      : "cargo fmt --manifest-path core/Cargo.toml -- --check",
  },
  {
    name: "cargo clippy",
    cmd: "cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used",
  },
  { name: "cargo test", cmd: "cargo test --manifest-path core/Cargo.toml" },
  {
    name: "format generated types",
    cmd: "npx prettier --write ui/types/generated",
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
