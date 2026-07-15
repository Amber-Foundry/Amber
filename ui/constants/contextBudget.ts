/**
 * Default context token budget for `debug_assemble_context` / `llm_chat`.
 * Must match `DEFAULT_ASSEMBLER_MAX_TOKENS` in `core/src/lib.rs`.
 */
export const CONTEXT_MAX_TOKENS = 8000 as const;

/** Privacy tier passed to the assembler (`cloud` vs `local` filtering). */
export type ContextAssemblerScope = "local" | "cloud";

// Old registry logic replaced by modelRegistry.ts
