import type { Conversation, Orbit } from "../types";

function compareId(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function newestFirst(left: Conversation, right: Conversation): number {
  const leftDate = Date.parse(left.createdAt);
  const rightDate = Date.parse(right.createdAt);
  const leftTime = Number.isFinite(leftDate) ? leftDate : -Infinity;
  const rightTime = Number.isFinite(rightDate) ? rightDate : -Infinity;
  return leftTime === rightTime ? compareId(left.id, right.id) : leftTime > rightTime ? -1 : 1;
}

/** Keep direct messages person-based and group conversations independently selectable. */
export function conversationSidebar(orbits: Orbit[], conversations: Conversation[]) {
  const uniqueConversations = new Map<string, Conversation>();
  for (const conversation of [...conversations].sort(newestFirst)) {
    if (!uniqueConversations.has(conversation.id)) uniqueConversations.set(conversation.id, conversation);
  }

  const directByOrbit = new Map<string, Conversation>();
  const groups: Conversation[] = [];
  for (const conversation of uniqueConversations.values()) {
    const participants = [...new Set(conversation.orbitIds)];
    if (participants.length > 1) groups.push(conversation);
    else if (participants.length === 1 && !directByOrbit.has(participants[0])) {
      directByOrbit.set(participants[0], conversation);
    }
  }

  const uniqueOrbits = new Map(orbits.map(orbit => [orbit.id, orbit]));
  const directMessages = [...uniqueOrbits.values()]
    .sort((left, right) => left.name.localeCompare(right.name, "en", { sensitivity: "base", numeric: true }) || compareId(left.id, right.id))
    .map(orbit => ({ orbit, conversation: directByOrbit.get(orbit.id) }));

  return { directMessages, groups };
}
