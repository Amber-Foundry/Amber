import { invoke } from "@tauri-apps/api/core";
import type {
  Backlink,
  Changeset,
  ChangesetItem,
  Door,
  DoorCreateInput,
  Node,
  NodeCreateInput,
  NodeUpdateInput,
  OnboardingNodeCommitInput,
  OnboardingProposedNode,
  Tag,
  TagCreateInput,
  Vault,
  VaultCreateInput,
  VaultUpdateInput,
  ChangesetCommitInput,
} from "./types/generated";
import { getMockInvoker } from "./ipcMockState.ts";

export type IpcResult<T> = { ok: T } | { err: string };
export type ChatMessage = {
  id: string;
  role: string;
  content: string;
  created_at: string;
  isStreaming?: boolean;
};
export type {
  Backlink,
  Changeset,
  ChangesetItem,
  Door,
  DoorCreateInput,
  Node,
  NodeCreateInput,
  NodeUpdateInput,
  OnboardingNodeCommitInput,
  OnboardingProposedNode,
  Tag,
  TagCreateInput,
  Vault,
  VaultCreateInput,
  VaultUpdateInput,
  ChangesetCommitInput,
};

async function invokeTyped<T>(
  command: string,
  payload?: Record<string, unknown>
): Promise<IpcResult<T>> {
  const mockInvoker = getMockInvoker();
  if (mockInvoker) {
    try {
      return (await mockInvoker(command, payload)) as IpcResult<T>;
    } catch (error) {
      return { err: String(error) };
    }
  }
  try {
    return await invoke<IpcResult<T>>(command, payload);
  } catch (error) {
    return { err: String(error) };
  }
}

export function greet(name: string) {
  return invokeTyped<string>("greet", { name });
}

export function dbPing() {
  return invokeTyped<string>("db_ping");
}

export function settingsGet(key: string) {
  return invokeTyped<string | null>("settings_get", { key });
}

export function settingsSet(key: string, value: string) {
  return invokeTyped<boolean>("settings_set", { key, value });
}

export function onboardingGetComplete() {
  return settingsGet("onboarding_complete");
}

export function onboardingSetComplete(isComplete: boolean) {
  return settingsSet("onboarding_complete", JSON.stringify(isComplete));
}

export function chatGetHistory(sessionId: string) {
  return invokeTyped<ChatMessage[]>("chat_get_history", { sessionId });
}

export function chatAppendMessage(id: string, role: string, content: string, sessionId: string) {
  return invokeTyped<void>("chat_append_message", { id, role, content, sessionId });
}

export function chatClearHistory(sessionId: string) {
  return invokeTyped<void>("chat_clear_history", { sessionId });
}

export function chatEditAndTruncate(
  editId: string,
  newContent: string,
  deleteIds: string[],
  sessionId: string
) {
  return invokeTyped<void>("chat_edit_and_truncate", { editId, newContent, deleteIds, sessionId });
}

export function vaultCreate(input: VaultCreateInput) {
  return invokeTyped<Vault>("vault_create", { input });
}

export function vaultList() {
  return invokeTyped<Vault[]>("vault_list");
}

export function vaultDelete(vaultId: string) {
  return invokeTyped<boolean>("vault_delete", { vaultId });
}

export function vaultUpdate(input: VaultUpdateInput) {
  return invokeTyped<Vault>("vault_update", { input });
}

export function vaultUpdatePosition(vaultId: string, x: number, y: number) {
  return invokeTyped<boolean>("vault_update_position", { vaultId, x, y });
}

export function vaultUpdateColorTheme(vaultId: string, colorTheme: string) {
  return invokeTyped<boolean>("vault_update_color_theme", { vaultId, colorTheme });
}

export function vaultGet(vaultId: string) {
  return invokeTyped<Vault | null>("vault_get", { vaultId });
}

export function nodeCreate(input: NodeCreateInput) {
  return invokeTyped<Node>("node_create", { input });
}

export function nodeGet(nodeId: string) {
  return invokeTyped<Node | null>("node_get", { nodeId });
}

export function nodeList() {
  return invokeTyped<Node[]>("node_list");
}

export function nodeUpdate(input: NodeUpdateInput) {
  return invokeTyped<Node>("node_update", { input });
}

export function nodeDelete(nodeId: string) {
  return invokeTyped<boolean>("node_delete", { nodeId });
}

export function nodeTouch(nodeId: string) {
  return invokeTyped<boolean>("node_touch", { nodeId });
}

export function tagList() {
  return invokeTyped<Tag[]>("tag_list");
}

export function tagCreate(input: TagCreateInput) {
  return invokeTyped<Tag>("tag_create", { input });
}

export function nodeTagsGet(nodeId: string) {
  return invokeTyped<Tag[]>("node_tags_get", { nodeId });
}

export function nodeTagAdd(nodeId: string, tagId: string) {
  return invokeTyped<boolean>("node_tag_add", { nodeId, tagId });
}

export function nodeTagRemove(nodeId: string, tagId: string) {
  return invokeTyped<boolean>("node_tag_remove", { nodeId, tagId });
}

export function doorCreate(input: DoorCreateInput) {
  return invokeTyped<Door>("door_create", { input });
}

export function doorListOutgoing(nodeId: string) {
  return invokeTyped<Door[]>("door_list_outgoing", { nodeId });
}

export function doorListIncoming(nodeId: string) {
  return invokeTyped<Backlink[]>("door_list_incoming", { nodeId });
}

export function doorListAll() {
  return invokeTyped<Door[]>("door_list_all");
}

export function doorDelete(doorId: string) {
  return invokeTyped<boolean>("door_delete", { doorId });
}

export function doorRepoint(doorId: string, targetNodeId: string) {
  return invokeTyped<boolean>("door_repoint", { doorId, targetNodeId });
}

export function authIsSetup() {
  return invokeTyped<boolean>("auth_secret_is_setup");
}

export function authSetPassword(password: string) {
  return invokeTyped<boolean>("auth_secret_set", { passphrase: password });
}

export function authVerifyPassword(password: string) {
  return invokeTyped<boolean>("auth_secret_verify", { passphrase: password });
}

export function priorityRefreshAll() {
  return invokeTyped<number>("priority_refresh_all");
}

export function priorityOptimizeAll() {
  return invokeTyped<number>("priority_optimize_all");
}

export function debugAssembleContext(nodeIds: string[], scope: string) {
  return invokeTyped<string>("debug_assemble_context", { nodeIds, scope });
}

export async function countTokens(text: string): Promise<number> {
  const result = await invokeTyped<number>("llm_count_tokens", { text });
  if ("ok" in result) {
    return result.ok;
  }
  throw new Error(result.err);
}

export function listLlmModels(provider: string, endpoint: string) {
  return invokeTyped<string[]>("llm_list_models", { provider, endpoint });
}

export function chatWithLlm(
  nodeIds: string[],
  scope: string,
  provider: string,
  endpoint: string,
  model: string,
  userPrompt: string,
  chartsEnabled: boolean,
  isRedactedUnlocked: boolean,
  sessionId: string
) {
  return invoke<string>("llm_chat", {
    nodeIds,
    scope,
    provider,
    endpoint,
    model,
    userPrompt,
    chartsEnabled,
    isRedactedUnlocked,
    sessionId,
  })
    .then((ok) => ({ ok }) as IpcResult<string>)
    .catch((error) => ({ err: String(error) }) as IpcResult<string>);
}

/** One-shot onboarding extraction: Q&A JSON → proposed nodes (calls configured LLM). */
export function onboardingExtractProposals(
  answersJson: string,
  provider: string,
  endpoint: string,
  model: string
) {
  return invoke<OnboardingProposedNode[]>("onboarding_extract_proposals", {
    answersJson,
    provider,
    endpoint,
    model,
  })
    .then((ok) => ({ ok }) as IpcResult<OnboardingProposedNode[]>)
    .catch((error) => ({ err: String(error) }) as IpcResult<OnboardingProposedNode[]>);
}

/** Persist accepted onboarding proposals into nodes and mark onboarding complete. */
export function onboardingCommit(proposals: OnboardingNodeCommitInput[]) {
  return invokeTyped<boolean>("onboarding_commit", { proposals });
}

export function saveMarkdownFile(defaultName: string, content: string) {
  return invokeTyped<boolean>("save_markdown_file", { defaultName, content });
}

export function memoryExtract(provider: string, endpoint: string, model: string) {
  return invoke<Changeset>("memory_extract", { provider, endpoint, model })
    .then((ok) => ({ ok }) as IpcResult<Changeset>)
    .catch((error) => ({ err: String(error) }) as IpcResult<Changeset>);
}

export function memoryExtractIfReady(
  provider: string,
  endpoint: string,
  model: string,
  sessionId: string
) {
  return invoke<Changeset | null>("memory_extract_if_ready", {
    provider,
    endpoint,
    model,
    sessionId,
  })
    .then((ok) => ({ ok }) as IpcResult<Changeset | null>)
    .catch((error) => ({ err: String(error) }) as IpcResult<Changeset | null>);
}

export function changesetCountPending() {
  return invoke<number>("changeset_count_pending")
    .then((ok) => ({ ok }) as IpcResult<number>)
    .catch((error) => ({ err: String(error) }) as IpcResult<number>);
}

export function changesetListPending() {
  return invoke<Changeset[]>("changeset_list_pending")
    .then((ok) => ({ ok }) as IpcResult<Changeset[]>)
    .catch((error) => ({ err: String(error) }) as IpcResult<Changeset[]>);
}

export function changesetListItems(changesetId: string) {
  return invoke<ChangesetItem[]>("changeset_list_items", { changesetId })
    .then((ok) => ({ ok }) as IpcResult<ChangesetItem[]>)
    .catch((error) => ({ err: String(error) }) as IpcResult<ChangesetItem[]>);
}

export function changesetCommit(input: ChangesetCommitInput) {
  return invoke<boolean>("changeset_commit", { input })
    .then((ok) => ({ ok }) as IpcResult<boolean>)
    .catch((error) => ({ err: String(error) }) as IpcResult<boolean>);
}

export function changesetListResolved() {
  return invoke<Changeset[]>("changeset_list_resolved")
    .then((ok) => ({ ok }) as IpcResult<Changeset[]>)
    .catch((error) => ({ err: String(error) }) as IpcResult<Changeset[]>);
}

export function debugSeedChangeset() {
  return invoke<boolean>("debug_seed_changeset")
    .then((ok) => ({ ok }) as IpcResult<boolean>)
    .catch((error) => ({ err: String(error) }) as IpcResult<boolean>);
}

export function memoryExtractForce(
  provider: string,
  endpoint: string,
  model: string
): Promise<IpcResult<Changeset>> {
  return invoke<Changeset>("memory_extract_force", {
    provider,
    endpoint,
    model,
  })
    .then((ok) => ({ ok }) as IpcResult<Changeset>)
    .catch((error) => ({ err: String(error) }) as IpcResult<Changeset>);
}

export function chatSetOffTheRecord(enabled: boolean) {
  return invokeTyped<boolean>("chat_set_off_the_record", { enabled });
}

export function chatIsOffTheRecord() {
  return invokeTyped<boolean>("chat_is_off_the_record");
}

export function chatConvertTemporaryToMemory(
  provider: string,
  endpoint: string,
  model: string
): Promise<IpcResult<Changeset>> {
  return invoke<Changeset>("chat_convert_temporary_to_memory", {
    provider,
    endpoint,
    model,
  })
    .then((ok) => ({ ok }) as IpcResult<Changeset>)
    .catch((error) => ({ err: String(error) }) as IpcResult<Changeset>);
}
