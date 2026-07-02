import { spawn } from "node:child_process";
import process from "node:process";
import { runBannedPatternChecks } from "./check-banned-patterns.mjs";

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
  console.log(`Amber preflight checks

Usage:
  npm run preflight
  npm run preflight -- --fix

What it runs:
  - Prettier + ESLint + TypeScript (UI)
  - Frontend + backend license checks
  - npm audit (warn-only locally; enforced in CI)
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

function runCapture(command, { cwd } = {}) {
  return new Promise((resolve) => {
    const child = spawn(command, {
      cwd,
      shell: true,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let combined = "";
    child.stdout.on("data", (chunk) => {
      combined += chunk;
    });
    child.stderr.on("data", (chunk) => {
      combined += chunk;
    });
    child.on("exit", (code) => resolve({ code: code ?? 1, combined }));
    child.on("error", () => resolve({ code: 1, combined: "" }));
  });
}

async function isCommandAvailable(cmd) {
  return new Promise((resolve) => {
    const checkCmd = process.platform === "win32" ? `where ${cmd}` : `which ${cmd}`;
    const child = spawn(checkCmd, { shell: true, stdio: "ignore" });
    child.on("exit", (code) => resolve(code === 0));
  });
}

async function runBannedPatterns() {
  return runBannedPatternChecks();
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
    name: "frontend build",
    cmd: async () => {
      console.log(
        "\n[Tip] Windows Key + Ctrl + Shift + B shortcut immediately resets your graphics driver and refreshes your display.\n" +
          "      It is a quick troubleshooting step when your screen freezes, goes black, or shows visual glitches.\n"
      );
      return run("npm run build");
    },
  },
  {
    name: "frontend tests",
    cmd: "npm run test:all",
  },
  {
    name: "frontend license check",
    cmd: "npm run license-check",
  },
  {
    name: "npm audit (warn-only)",
    cmd: async () => {
      const { code, combined } = await runCapture("npm audit --audit-level=high");
      if (code !== 0) {
        console.warn(
          "\n[Warning] npm audit reported high/critical issues (preflight continues).\n" +
            "   This check is enforced in CI via .github/workflows/security.yml.\n"
        );
        const lines = combined.split(/\r?\n/).filter((line) => line.trim());
        for (const line of lines.slice(-20)) {
          console.warn(line);
        }
      }
      return 0;
    },
  },
  {
    name: "backend cargo-deny check",
    cmd: async () => {
      if (!(await isCommandAvailable("cargo-deny"))) {
        console.warn(
          "\n[Warning] cargo-deny is not installed. Backend license/security checks will be skipped locally.\n" +
            "   To install cargo-deny locally, run: cargo install --locked cargo-deny\n" +
            "   Note: This check is enforced in CI via .github/workflows/security.yml.\n"
        );
        return 0;
      }

      const { code, combined } = await runCapture(
        "cargo deny --manifest-path core/Cargo.toml check --show-stats"
      );
      const lines = combined.split(/\r?\n/);

      if (code === 0) {
        for (const line of lines) {
          if (/^\s*(advisories|bans|licenses|sources)\s+(ok|FAILED):/.test(line)) {
            console.log(line.trimEnd());
          }
        }
        return 0;
      }

      const errors = lines.filter((line) => /\berror\[/.test(line));
      if (errors.length > 0) {
        for (const line of errors) {
          console.error(line);
        }
      } else {
        for (const line of lines.filter((line) => line.trim()).slice(-12)) {
          console.error(line);
        }
      }
      return code;
    },
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
    name: "format generated types",
    cmd: "npx prettier --write --ignore-path .prettierignore.none ui/types/generated",
  },
  {
    name: "refresh generated types index",
    cmd: "git add ui/types/generated && git reset HEAD -- ui/types/generated",
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
