const mockLocalStorage: Record<string, string> = {};
const dispatchedEvents: Array<{ type: string; detail?: unknown }> = [];

class CustomEventMock {
  type: string;
  detail?: unknown;
  constructor(type: string, options?: { detail?: unknown }) {
    this.type = type;
    this.detail = options?.detail;
  }
}

(globalThis as unknown as Record<string, unknown>).CustomEvent = CustomEventMock;

globalThis.window = {
  localStorage: {
    getItem: (key: string) => mockLocalStorage[key] ?? null,
    setItem: (key: string, value: string) => {
      mockLocalStorage[key] = value;
    },
    removeItem: (key: string) => {
      delete mockLocalStorage[key];
    },
    clear: () => {
      for (const key in mockLocalStorage) {
        delete mockLocalStorage[key];
      }
    },
  },
  dispatchEvent: (event: { type: string; detail?: unknown }) => {
    dispatchedEvents.push(event);
    return true;
  },
  CustomEvent: CustomEventMock,
} as unknown as Window & typeof globalThis;

import { runPrivacyTests } from "../ui/utils/privacy.ts";
import { AppError, toAppError, unwrapIpcResult } from "../ui/services/ipcResult.ts";
import { resolveVaultPath, updateVaultPosition } from "../ui/services/vaults.ts";
import { sanitizeSvgText, sanitizeSvg } from "../ui/utils/svgSanitizer.ts";
import { setMockInvoker } from "../ui/ipc.ts";
import { preprocessWikiLinks } from "../ui/utils/wikilinkUtils.ts";
import {
  getLlmMode,
  setLlmMode,
  getPlantUmlServer,
  setPlantUmlServer,
} from "../ui/utils/settings.ts";
import type { Node, Vault } from "../ui/ipc.ts";

function runDoorServiceTests() {
  const assertAppError = (val: unknown, expectedMessage: string, testName: string) => {
    const res = toAppError(val);
    if (!(res instanceof AppError)) {
      throw new Error(`${testName} Failed: Expected result to be an instance of AppError`);
    }
    if (res.message !== expectedMessage) {
      throw new Error(
        `${testName} Failed: Expected message '${expectedMessage}', got '${res.message}'`
      );
    }
  };

  // Test 1: toAppError with an existing AppError
  const appErr = new AppError("Already an AppError");
  const res1 = toAppError(appErr);
  if (res1 !== appErr) {
    throw new Error("toAppError Test 1 Failed: Expected identical instance reference");
  }

  // Test 2: toAppError with standard built-in Error
  assertAppError(
    new Error("Standard built-in error message"),
    "Standard built-in error message",
    "Test 2"
  );

  // Test 3: toAppError with primitive string value
  assertAppError("Raw string error", "Raw string error", "Test 3");

  // Test 4: toAppError with an arbitrary object
  assertAppError({ foo: "bar" }, "[object Object]", "Test 4");

  // Test 5: toAppError with custom subclass of Error
  class CustomError extends Error {
    constructor(msg: string) {
      super(msg);
      this.name = "CustomError";
    }
  }
  assertAppError(new CustomError("My custom error"), "My custom error", "Test 5");

  // Test 6: toAppError with object containing .message property
  assertAppError(
    { message: "Object with message property" },
    "Object with message property",
    "Test 6"
  );

  // Test 7: toAppError with object containing .error property
  assertAppError({ error: "Object with error property" }, "Object with error property", "Test 7");

  // Test 8: toAppError with object containing non-string .message property
  assertAppError({ message: 999 }, "999", "Test 8");

  // Test 9: toAppError with object containing non-string .error property
  assertAppError({ error: true }, "true", "Test 9");

  // Test 10: toAppError with primitive number value
  assertAppError(500, "500", "Test 10");

  // Test 11: toAppError with primitive boolean value
  assertAppError(false, "false", "Test 11");

  // Test 12: toAppError with null
  assertAppError(null, "null", "Test 12");

  // Test 13: toAppError with undefined
  assertAppError(undefined, "undefined", "Test 13");

  // Test 14: toAppError with an array
  assertAppError(["error1", "error2"], "error1,error2", "Test 14");

  // Test 15: toAppError with standard Error with empty/missing message
  assertAppError(new Error(""), "Unknown Error", "Test 15");
}

function runVaultServiceTests() {
  const assertResolvedPath = (
    node: Partial<Node>,
    vaults: Partial<Vault>[],
    expected: string,
    testName: string
  ) => {
    const res = resolveVaultPath(node as Node, vaults as Vault[]);
    if (res !== expected) {
      throw new Error(`${testName} Failed: Expected '${expected}', got '${res}'`);
    }
  };

  // Setup mock vaults
  const vault1: Partial<Vault> = { id: "v1", name: "Work", parentVaultId: undefined };
  const vault2: Partial<Vault> = { id: "v2", name: "Projects", parentVaultId: "v1" };
  const vault3 = { id: "v3", name: "MindVault", parent_vault_id: "v1" } as unknown as Vault; // snake_case mock
  const vault4: Partial<Vault> = { id: "v4", name: "DeadEnd", parentVaultId: "nonexistent" };

  // Case A.1: subVaultId set, both exist
  assertResolvedPath(
    { vaultId: "v1", subVaultId: "v2" },
    [vault1, vault2],
    "Work / Projects",
    "resolveVaultPath Test A.1"
  );

  // Case A.2: subVaultId set, only child exists
  assertResolvedPath(
    { vaultId: "nonexistent", subVaultId: "v2" },
    [vault2],
    "Projects",
    "resolveVaultPath Test A.2"
  );

  // Case A.3: subVaultId set, only parent exists
  assertResolvedPath(
    { vaultId: "v1", subVaultId: "nonexistent" },
    [vault1],
    "Work",
    "resolveVaultPath Test A.3"
  );

  // Case A.4: subVaultId set, neither exists
  assertResolvedPath(
    { vaultId: "nonexistent", subVaultId: "nonexistent" },
    [],
    "Unknown Vault",
    "resolveVaultPath Test A.4"
  );

  // Case B.1: subVaultId null, vaultId nonexistent
  assertResolvedPath(
    { vaultId: "nonexistent", subVaultId: null },
    [],
    "Unknown Vault",
    "resolveVaultPath Test B.1"
  );

  // Case B.2: subVaultId null, vaultId exists, no parent
  assertResolvedPath(
    { vaultId: "v1", subVaultId: null },
    [vault1],
    "Work",
    "resolveVaultPath Test B.2"
  );

  // Case B.3: subVaultId null, vaultId exists, parentVaultId exists (camelCase)
  assertResolvedPath(
    { vaultId: "v2", subVaultId: null },
    [vault1, vault2],
    "Work / Projects",
    "resolveVaultPath Test B.3"
  );

  // Case B.4: subVaultId null, vaultId exists, parent_vault_id exists (snake_case)
  assertResolvedPath(
    { vaultId: "v3", subVaultId: null },
    [vault1, vault3],
    "Work / MindVault",
    "resolveVaultPath Test B.4"
  );

  // Case B.5: subVaultId null, vaultId exists, parentVaultId nonexistent in list
  assertResolvedPath(
    { vaultId: "v4", subVaultId: null },
    [vault4],
    "DeadEnd",
    "resolveVaultPath Test B.5"
  );

  // Case C.1: Test with pre-computed Map
  const mockMap = new Map<string, Vault>();
  mockMap.set("v1", vault1 as Vault);
  mockMap.set("v2", vault2 as Vault);
  const resMap = resolveVaultPath({ vaultId: "v1", subVaultId: "v2" } as Node, mockMap);
  if (resMap !== "Work / Projects") {
    throw new Error(
      `resolveVaultPath Test C.1 Failed: Expected 'Work / Projects', got '${resMap}'`
    );
  }
}

function runSvgSanitizerTests() {
  const assertSanitized = (input: string, expected: string, testName: string) => {
    const res = sanitizeSvgText(input);
    if (res !== expected) {
      throw new Error(`${testName} Failed: Expected '${expected}', got '${res}'`);
    }
  };

  // Test 1: Basic alphanumeric with spaces and hyphens
  assertSanitized("Hello World 123-456", "Hello World 123-456", "Test 1");

  // Test 2: HTML script tags (XSS check)
  assertSanitized("<script>alert('XSS')</script>", "scriptalertXSSscript", "Test 2");

  // Test 3: Standard special characters (should be stripped)
  assertSanitized("Hello! @World# & %^*()_+={}[]|\\:;\"'<>,.?/~`", "Hello World  ", "Test 3");

  // Test 4: Empty string
  assertSanitized("", "", "Test 4");

  // Test 5: Unicode characters (should be stripped due to non-alphanumeric ASCII regex)
  assertSanitized("Hello 世界!", "Hello ", "Test 5");

  // Test 6: Emoji characters (should be stripped)
  assertSanitized("Hello 🚀!", "Hello ", "Test 6");

  // Only test sanitizeSvg in environments where DOMParser is defined
  if (typeof globalThis.DOMParser !== "undefined") {
    const dirtySvg = `<svg><script>alert('XSS')</script><rect onclick="alert('XSS')" href="javascript:alert('XSS')" xlink:href="data:image/svg+xml;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==" src="vbscript:msgbox('XSS')" width="100"/></svg>`;
    const sanitized = sanitizeSvg(dirtySvg);
    if (
      sanitized.includes("<script") ||
      sanitized.includes("onclick") ||
      sanitized.includes("javascript:") ||
      sanitized.includes("data:") ||
      sanitized.includes("vbscript:")
    ) {
      throw new Error(
        "runSvgSanitizerTests Failed: sanitizeSvg failed to strip script elements, event handlers, or javascript:/data:/vbscript: attributes"
      );
    }
  }
}

async function runIpcResultTests() {
  // Test 1: Successful path
  const successPromise = Promise.resolve({ ok: "Hello IPC" });
  const successRes = await unwrapIpcResult(successPromise);
  if (successRes !== "Hello IPC") {
    throw new Error(`unwrapIpcResult Test 1 Failed: Expected 'Hello IPC', got '${successRes}'`);
  }

  // Test 2: Error path
  const errorPromise = Promise.resolve({ err: "Something broke in Rust core" });
  try {
    await unwrapIpcResult(errorPromise);
    throw new Error(
      "unwrapIpcResult Test 2 Failed: Expected promise to reject but it resolved successfully"
    );
  } catch (err) {
    if (!(err instanceof AppError)) {
      throw new Error(
        `unwrapIpcResult Test 2 Failed: Expected error to be AppError, got '${String(err)}'`
      );
    }
    if (err.message !== "Something broke in Rust core") {
      throw new Error(
        `unwrapIpcResult Test 2 Failed: Expected message 'Something broke in Rust core', got '${err.message}'`
      );
    }
  }
}

async function runVaultPositionDebounceTests() {
  const invocations: Array<{ command: string; payload: Record<string, unknown> | undefined }> = [];

  // Register mock invoker to log IPC calls
  setMockInvoker(async (command, payload) => {
    invocations.push({ command, payload });
    return { ok: true };
  });

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  // Case 1: Single position update triggers after 300ms
  updateVaultPosition("vault-100", 10, 20);
  if (invocations.length !== 0) {
    throw new Error("updateVaultPosition debounce failed: invoked immediately before delay");
  }

  await sleep(150);
  if (invocations.length !== 0) {
    throw new Error("updateVaultPosition debounce failed: invoked prematurely at 150ms");
  }

  await sleep(250); // 400ms total
  if (invocations.length !== 1) {
    throw new Error(
      `updateVaultPosition debounce failed: expected exactly 1 invocation, got ${invocations.length}`
    );
  }
  if (invocations[0].command !== "vault_update_position") {
    throw new Error(
      `updateVaultPosition debounce failed: expected command 'vault_update_position', got '${invocations[0].command}'`
    );
  }
  if (
    invocations[0].payload?.vaultId !== "vault-100" ||
    invocations[0].payload?.x !== 10 ||
    invocations[0].payload?.y !== 20
  ) {
    throw new Error(
      `updateVaultPosition debounce failed: incorrect payload parameters, got ${JSON.stringify(invocations[0].payload)}`
    );
  }

  // Clear tracked invocations
  invocations.length = 0;

  // Case 2: Rapid sequential position updates are correctly debounced (only last one succeeds)
  updateVaultPosition("vault-200", 5, 5);
  await sleep(100);
  updateVaultPosition("vault-200", 15, 15);
  await sleep(100);
  updateVaultPosition("vault-200", 25, 35); // final coordinates

  // At this point, no invocation should have happened yet (timer reset twice)
  if (invocations.length !== 0) {
    throw new Error(
      "updateVaultPosition debounce failed: premature invocation during rapid updates"
    );
  }

  // Wait 150ms (total 350ms since first call, but only 150ms since last call)
  await sleep(150);
  if (invocations.length !== 0) {
    throw new Error(
      "updateVaultPosition debounce failed: premature invocation 150ms after final update"
    );
  }

  // Wait another 250ms (total 400ms since final call)
  await sleep(250);
  if (invocations.length !== 1) {
    throw new Error(
      `updateVaultPosition debounce failed: expected exactly 1 debounced call, got ${invocations.length}`
    );
  }
  if (
    invocations[0].payload?.vaultId !== "vault-200" ||
    invocations[0].payload?.x !== 25 ||
    invocations[0].payload?.y !== 35
  ) {
    throw new Error(
      `updateVaultPosition debounce failed: incorrect final payload parameters, got ${JSON.stringify(invocations[0].payload)}`
    );
  }

  // Clean up mock invoker for other tests
  setMockInvoker(null);
}

function runSettingsTests() {
  // Clear mock local storage before starting
  window.localStorage.clear();

  // Test 1: Fallback case (when local storage is empty)
  const mode1 = getLlmMode();
  if (mode1 !== "local") {
    throw new Error(`getLlmMode Test 1 Failed: Expected 'local', got '${mode1}'`);
  }

  // Test 2: Valid 'cloud' mode read
  window.localStorage.setItem("mindvault.llm.mode", "cloud");
  const mode2 = getLlmMode();
  if (mode2 !== "cloud") {
    throw new Error(`getLlmMode Test 2 Failed: Expected 'cloud', got '${mode2}'`);
  }

  // Test 3: Valid 'hybrid' mode read
  window.localStorage.setItem("mindvault.llm.mode", "hybrid");
  const mode3 = getLlmMode();
  if (mode3 !== "hybrid") {
    throw new Error(`getLlmMode Test 3 Failed: Expected 'hybrid', got '${mode3}'`);
  }

  // Test 4: Invalid fallback mode read
  window.localStorage.setItem("mindvault.llm.mode", "super-ai");
  const mode4 = getLlmMode();
  if (mode4 !== "local") {
    throw new Error(`getLlmMode Test 4 Failed: Expected 'local', got '${mode4}'`);
  }

  // Test 5: setLlmMode updates correctly
  setLlmMode("cloud");
  const mode5 = window.localStorage.getItem("mindvault.llm.mode");
  if (mode5 !== "cloud") {
    throw new Error(`setLlmMode Test 5 Failed: Expected 'cloud' in storage, got '${mode5}'`);
  }

  // Test 6: setLlmMode triggers dispatchEvent
  dispatchedEvents.length = 0;
  setLlmMode("hybrid");
  if (dispatchedEvents.length !== 1) {
    throw new Error(
      `setLlmMode Test 6 Failed: Expected 1 event dispatched, got ${dispatchedEvents.length}`
    );
  }
  if (dispatchedEvents[0].type !== "mindvault:llm-settings-changed") {
    throw new Error(
      `setLlmMode Test 6 Failed: Expected event type 'mindvault:llm-settings-changed', got '${dispatchedEvents[0].type}'`
    );
  }

  // Test 7: setLlmMode("local") synchronizes provider to 'ollama' if current is cloud (e.g. 'openai')
  window.localStorage.setItem("mindvault.llm.provider", "openai");
  setLlmMode("local");
  const localProvider = window.localStorage.getItem("mindvault.llm.provider");
  if (localProvider !== "ollama") {
    throw new Error(
      `setLlmMode Test 7 Failed: Expected synchronized local provider 'ollama', got '${localProvider}'`
    );
  }

  // Test 8: setLlmMode("cloud") synchronizes provider to 'openai' if current is local (e.g. 'ollama')
  window.localStorage.setItem("mindvault.llm.provider", "ollama");
  setLlmMode("cloud");
  const cloudProvider = window.localStorage.getItem("mindvault.llm.provider");
  if (cloudProvider !== "openai") {
    throw new Error(
      `setLlmMode Test 8 Failed: Expected synchronized cloud provider 'openai', got '${cloudProvider}'`
    );
  }

  // Test 9: getPlantUmlServer defaults to default if key not present
  const defaultServer = getPlantUmlServer();
  if (defaultServer !== "https://www.plantuml.com/plantuml") {
    throw new Error(`getPlantUmlServer Test 9 Failed: Expected default, got '${defaultServer}'`);
  }

  // Test 10: setPlantUmlServer trims and normalizes valid HTTPS URL
  setPlantUmlServer("  https://my-plantuml-server.com/puml/  ");
  const puml1 = getPlantUmlServer();
  if (puml1 !== "https://my-plantuml-server.com/puml") {
    throw new Error(`setPlantUmlServer Test 10 Failed: Expected normalized URL, got '${puml1}'`);
  }

  // Test 11: setPlantUmlServer prepends https:// if protocol is missing
  setPlantUmlServer("local-plantuml:8080/puml");
  const puml2 = getPlantUmlServer();
  if (puml2 !== "https://local-plantuml:8080/puml") {
    throw new Error(`setPlantUmlServer Test 11 Failed: Expected prepended https, got '${puml2}'`);
  }

  // Test 12: setPlantUmlServer rejects invalid protocols (e.g. javascript:) and falls back to default
  setPlantUmlServer("javascript:alert(1)");
  const puml3 = getPlantUmlServer();
  if (puml3 !== "https://www.plantuml.com/plantuml") {
    throw new Error(
      `setPlantUmlServer Test 12 Failed: Expected fallback on invalid protocol, got '${puml3}'`
    );
  }

  // Test 13: setPlantUmlServer handles http:// correctly
  setPlantUmlServer("http://localhost:8080/puml/");
  const puml4 = getPlantUmlServer();
  if (puml4 !== "http://localhost:8080/puml") {
    throw new Error(`setPlantUmlServer Test 13 Failed: Expected HTTP URL kept, got '${puml4}'`);
  }

  // Reset/clean up
  window.localStorage.clear();
  dispatchedEvents.length = 0;
}

function runWikiLinksTests() {
  // Test 1: [[Title|id]] format
  const res1 = preprocessWikiLinks("[[My Node|node-123]]");
  if (res1 !== "[My Node](#node/node-123)") {
    throw new Error(
      `preprocessWikiLinks Test 1 Failed: Expected '[My Node](#node/node-123)', got '${res1}'`
    );
  }

  // Test 2: Standard [[Title]] format
  const res2 = preprocessWikiLinks("[[My Standard Node]]");
  if (res2 !== "[My Standard Node](#node/search:My%20Standard%20Node)") {
    throw new Error(
      `preprocessWikiLinks Test 2 Failed: Expected '[My Standard Node](#node/search:My%20Standard%20Node)', got '${res2}'`
    );
  }

  // Test 3: Multiple occurrences and mixed formats
  const mixed = "Check [[First Node|node-1]] and also [[Second Node]] in detail.";
  const res3 = preprocessWikiLinks(mixed);
  const expected =
    "Check [First Node](#node/node-1) and also [Second Node](#node/search:Second%20Node) in detail.";
  if (res3 !== expected) {
    throw new Error(`preprocessWikiLinks Test 3 Failed: Expected '${expected}', got '${res3}'`);
  }
}

try {
  runPrivacyTests();
  console.log("✓ All frontend privacy utility tests passed successfully!");
  runDoorServiceTests();
  console.log("✓ All doors/IPC error service utility tests passed successfully!");
  runVaultServiceTests();
  console.log("✓ All vaults service utility tests passed successfully!");
  runSvgSanitizerTests();
  console.log("✓ All SVG sanitizer utility tests passed successfully!");
  await runIpcResultTests();
  console.log("✓ All IPC result unwrapping utility tests passed successfully!");
  await runVaultPositionDebounceTests();
  console.log("✓ All vault position debounce utility tests passed successfully!");
  runWikiLinksTests();
  console.log("✓ All wikilink preprocessor utility tests passed successfully!");
  runSettingsTests();
  console.log("✓ All settings utility tests passed successfully!");
  process.exit(0);
} catch (err) {
  console.error("Frontend utility self-test failed:", err);
  process.exit(1);
}
