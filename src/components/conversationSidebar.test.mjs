import assert from "node:assert/strict";
import test from "node:test";
import { conversationSidebar } from "./conversationSidebar.ts";

const orbit = (id, name = id) => ({ id, name });
const conversation = (id, orbitIds, createdAt = "2026-11-21T12:00:00Z") => ({ id, orbitIds, createdAt, title: id });

test("direct messages list every Sprite and select its most recent direct conversation", () => {
  const orbits = [orbit("c", "Zora"), orbit("b", "Nova"), orbit("a", "Aster")];
  const conversations = [
    conversation("old-a", ["a"], "2026-11-19T12:00:00Z"),
    conversation("group", ["a", "b"]),
    conversation("new-a", ["a"], "2026-11-20T12:00:00Z"),
    conversation("b", ["b"]),
    conversation("orphan", ["deleted"]),
  ];
  const result = conversationSidebar(orbits, conversations);
  assert.deepEqual(result.directMessages.map(item => [item.orbit.id, item.conversation?.id]), [["a", "new-a"], ["b", "b"], ["c", undefined]]);
  assert.deepEqual(result.groups.map(item => item.id), ["group"]);
  assert.equal(result.directMessages[0].orbit, orbits[2]);
  assert.equal(result.directMessages[0].conversation, conversations[2]);
  assert.deepEqual(orbits.map(item => item.id), ["c", "b", "a"]);
  assert.deepEqual(conversations.map(item => item.id), ["old-a", "group", "new-a", "b", "orphan"]);
});

test("group ordering and direct selection have stable ties and invalid date fallbacks", () => {
  const conversations = [
    conversation("z", ["a", "b"], "invalid"),
    conversation("c", ["a", "b"]),
    conversation("b", ["a", "b"]),
    conversation("a", ["a", "b"], "invalid"),
    conversation("dm-z", ["a"]),
    conversation("dm-a", ["a"]),
  ];
  const first = conversationSidebar([orbit("a")], conversations);
  const reversed = conversationSidebar([orbit("a")], [...conversations].reverse());
  assert.deepEqual(first.groups.map(item => item.id), ["b", "c", "a", "z"]);
  assert.equal(first.directMessages[0].conversation.id, "dm-a");
  assert.deepEqual(first, reversed);
});

test("duplicate records and repeated participants never create duplicate or mistaken group rows", () => {
  const rows = [
    conversation("same-group", ["a", "b"], "2026-11-20T12:00:00Z"),
    conversation("same-group", ["a", "b"], "2026-11-21T12:00:00Z"),
    conversation("repeated-dm", ["a", "a"]),
    conversation("empty", []),
  ];
  const result = conversationSidebar([orbit("a"), orbit("a")], rows);
  assert.equal(result.directMessages.length, 1);
  assert.equal(result.directMessages[0].conversation.id, "repeated-dm");
  assert.equal(result.groups.length, 1);
  assert.equal(result.groups[0], rows[1]);
});

test("empty workspaces stay empty and large teams are never truncated", () => {
  assert.deepEqual(conversationSidebar([], []), { directMessages: [], groups: [] });
  const orbits = Array.from({ length: 18 }, (_, index) => orbit(String(index), `Sprite ${index}`));
  const rows = conversationSidebar(orbits.reverse(), []).directMessages;
  assert.equal(rows.length, 18);
  assert.deepEqual(rows.map(item => item.orbit.name), Array.from({ length: 18 }, (_, index) => `Sprite ${index}`));
  const ties = conversationSidebar([orbit("z", "nova"), orbit("a", "Nova")], []).directMessages;
  assert.deepEqual(ties.map(item => item.orbit.id), ["a", "z"]);
});
