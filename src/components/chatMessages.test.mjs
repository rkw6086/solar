import assert from "node:assert/strict";
import test from "node:test";
import { mergeChatMessages } from "./chatMessages.ts";
const message = (id, content, conversationId = "chat") => ({ id, content, conversationId, role: "assistant", createdAt: "2026-11-19T00:00:00Z" });
test("an older persisted snapshot cannot overwrite newly streamed text", () => {
  assert.deepEqual(mergeChatMessages([message("1", "Old")], [message("1", "Old plus new tokens")], "chat"), [message("1", "Old plus new tokens")]);
});
test("new streamed messages are appended without duplicate IDs", () => {
  assert.deepEqual(mergeChatMessages([message("1", "one")], [message("2", "two"), message("2", "two updated")], "chat"), [message("1", "one"), message("2", "two updated")]);
});
test("late events from a previous conversation never cross into the next chat", () => {
  assert.deepEqual(mergeChatMessages([message("1", "new", "next")], [message("2", "old", "chat")], "next"), [message("1", "new", "next")]);
});
