import { fileURLToPath } from "node:url";
import process from "node:process";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";

function runCapture(command) {
  return new Promise((resolve) => {
    const child = spawn(command, {
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

function getRgCommand() {
  return getBundledRipgrepPath() ?? "rg";
}

async function assertRgNoMatches({ name, args }) {
  const cmd = args.join(" ");
  const { code } = await runCapture(cmd);
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

const BANNED_LOGGING_CREDENTIALS_REGEX =
  '"(tracing|log)::(trace|debug|info|warn|error)!\\([^\\n]*(api_key|password|secret|token)\\s*="';

export async function runBannedPatternChecks() {
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

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const code = await runBannedPatternChecks();
  process.exit(code);
}
