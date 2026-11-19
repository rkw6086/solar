import test from "node:test";
import assert from "node:assert/strict";
import { orbitProfileNeedsGeneration } from "./orbitProfile.ts";

const original = {
  name: "Nova", role: "Researcher", personality: "Curious",
  tasks: "Check sources", instructions: "Private generated prompt", model: "one", providerId: "a",
};

test("only user profile text requests a new generation", () => {
  assert(orbitProfileNeedsGeneration(original));
  assert(!orbitProfileNeedsGeneration({ ...original, model: "two", providerId: "b", autonomy: "trusted", avatarSeed: "new-face", projectIds: ["new"], instructions: "ignored" }, original));
  for (const key of ["name", "role", "personality", "tasks"]) {
    assert(orbitProfileNeedsGeneration({ ...original, [key]: original[key] + " changed" }, original), key);
    assert(!orbitProfileNeedsGeneration({ ...original, [key]: ` ${original[key]} ` }, original), `${key} whitespace`);
  }
});

test("legacy missing task fields do not force model-only edits to regenerate", () => {
  const legacy = { ...original, tasks: undefined };
  assert(!orbitProfileNeedsGeneration({ ...legacy, tasks: "", model: "two" }, legacy));
  assert(orbitProfileNeedsGeneration({ ...legacy, tasks: "New tasks" }, legacy));
});
