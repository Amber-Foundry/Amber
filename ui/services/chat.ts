import {
  chatAppendMessage as ipcChatAppendMessage,
  chatClearHistory as ipcChatClearHistory,
  chatGetHistory as ipcChatGetHistory,
  chatEditAndTruncate as ipcChatEditAndTruncate,
  chatSetOffTheRecord as ipcChatSetOffTheRecord,
  chatIsOffTheRecord as ipcChatIsOffTheRecord,
  type ChatMessage,
} from "../ipc";
import { unwrapIpcResult } from "./ipcResult";

export type { ChatMessage };

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
