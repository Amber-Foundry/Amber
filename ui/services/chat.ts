import {
  chatAppendMessage as ipcChatAppendMessage,
  chatClearHistory as ipcChatClearHistory,
  chatGetHistory as ipcChatGetHistory,
  chatEditAndTruncate as ipcChatEditAndTruncate,
  chatSetOffTheRecord as ipcChatSetOffTheRecord,
  chatIsOffTheRecord as ipcChatIsOffTheRecord,
  chatListSessions as ipcChatListSessions,
  chatCreateSession as ipcChatCreateSession,
  chatDeleteSession as ipcChatDeleteSession,
  chatUpdateSessionSummary as ipcChatUpdateSessionSummary,
  chatClearEphemeralSession,
  type ChatMessage,
  type ChatSession,
} from "../ipc";
import { unwrapIpcResult } from "./ipcResult";

export type { ChatMessage, ChatSession };

export async function getChatHistory(sessionId: string): Promise<ChatMessage[]> {
  return unwrapIpcResult(ipcChatGetHistory(sessionId));
}

export async function chatAppendMessage(
  id: string,
  role: string,
  content: string,
  sessionId: string
): Promise<void> {
  return unwrapIpcResult(ipcChatAppendMessage(id, role, content, sessionId));
}

export async function clearChatHistory(sessionId: string): Promise<void> {
  return unwrapIpcResult(ipcChatClearHistory(sessionId));
}

export async function chatEditAndTruncate(
  editId: string,
  newContent: string,
  deleteIds: string[],
  sessionId: string
): Promise<void> {
  return unwrapIpcResult(ipcChatEditAndTruncate(editId, newContent, deleteIds, sessionId));
}

export async function chatSetOffTheRecord(enabled: boolean): Promise<boolean> {
  return unwrapIpcResult(ipcChatSetOffTheRecord(enabled));
}

export async function chatIsOffTheRecord(): Promise<boolean> {
  return unwrapIpcResult(ipcChatIsOffTheRecord());
}

export async function chatListSessions(): Promise<ChatSession[]> {
  return unwrapIpcResult(ipcChatListSessions());
}

export async function chatCreateSession(id: string, summary?: string): Promise<void> {
  return unwrapIpcResult(ipcChatCreateSession(id, summary));
}

export async function chatDeleteSession(id: string): Promise<void> {
  try {
    await chatClearEphemeralSession(id);
  } catch (err) {
    console.warn(`[chat] Failed clearing ephemeral session ${id}:`, err);
  }
  return unwrapIpcResult(ipcChatDeleteSession(id));
}

export async function chatUpdateSessionSummary(id: string, summary: string): Promise<void> {
  return unwrapIpcResult(ipcChatUpdateSessionSummary(id, summary));
}
