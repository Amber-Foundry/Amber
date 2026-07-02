import { vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom";
import { setMockInvoker } from "../ipcMockState";

// Safety net: Mock Tauri native API core module
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// Reset IPC mock invoker between test runs to avoid pollution
beforeEach(() => {
  setMockInvoker(null);
});

afterEach(() => {
  setMockInvoker(null);
});
