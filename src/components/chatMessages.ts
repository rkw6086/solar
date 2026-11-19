import type { Message } from "../types";

// Streaming events can be newer than the persisted 150 ms snapshot. Keep the
// event content when a concurrent history refresh finishes a little later.
export function mergeChatMessages(stored: Message[], live: Iterable<Message>, conversationId: string): Message[] {
  const merged = new Map(stored.filter(message => message.conversationId === conversationId).map(message => [message.id, message]));
  for (const message of live) {
    if (message.conversationId === conversationId) merged.set(message.id, message);
  }
  return [...merged.values()];
}
