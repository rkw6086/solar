import assert from "node:assert/strict";
import test from "node:test";
import { generateOrbitAvatarSeed, generateOrbitName, generateOrbitPersonality, orbitAppearance } from "./orbitIdentity.ts";

test("name suggestions avoid current names regardless of case and whitespace", () => {
  assert.notEqual(generateOrbitName([" nova "], () => 0).toLowerCase(), "nova");
  const names = [];
  for (let i = 0; i < 80; i += 1) names.push(generateOrbitName(names, () => 0));
  assert.equal(new Set(names).size, 80);
  assert(names.every((name) => name.length <= 100));
});

test("personality suggestions combine readable temperament, approach, and a quirk", () => {
  const calm = generateOrbitPersonality(() => 0);
  const other = generateOrbitPersonality(() => 0.999);
  assert.match(calm, /Warm and encouraging;/);
  assert.match(calm, /questions/);
  assert.match(calm, /dry wit/);
  assert.notEqual(calm, other);
  assert(calm.length < 8000);
});

test("faces stay stable and an eighteen-Sprite team has varied eyes and smiles", () => {
  const names = Array.from({length: 18}, (_, i) => `orbit-${i}`);
  const looks = names.map((name) => orbitAppearance(name, "#abcdef"));
  assert.deepEqual(looks, names.map((name) => orbitAppearance(name, "#abcdef")));
  assert(new Set(looks.map((look) => look.eyes)).size >= 4);
  assert(new Set(looks.map((look) => look.smile)).size >= 4);
  assert(new Set(looks.map((look) => JSON.stringify(look))).size >= 12);
});

test("regenerating an avatar changes its face and remains stable afterwards", () => {
  const identity = "sprite-123";
  const color = "#83b6ab";
  const first = generateOrbitAvatarSeed(identity, color, () => "new-avatar");
  assert.notEqual(first, identity);
  assert.notDeepEqual(orbitAppearance(first, color), orbitAppearance(identity, color));
  assert.deepEqual(orbitAppearance(first, color), orbitAppearance(first, color));
  const second = generateOrbitAvatarSeed(first, color, () => "new-avatar");
  assert.notEqual(second, first);
  assert.notDeepEqual(orbitAppearance(second, color), orbitAppearance(first, color));
});

test("avatar regeneration retries a seed that would repeat the same face", () => {
  const identity = "same-seed";
  const generated = generateOrbitAvatarSeed(identity, "#83b6ab", () => identity);
  assert.notEqual(generated, identity);
  assert.notDeepEqual(orbitAppearance(generated, "#83b6ab"), orbitAppearance(identity, "#83b6ab"));
});
