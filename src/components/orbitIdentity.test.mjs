import assert from "node:assert/strict";
import test from "node:test";
import { eyeStyles, smileStyles, planetStyles, generateOrbitAvatarSeed, generateOrbitName, generateOrbitPersonality, orbitAppearance } from "./orbitIdentity.ts";

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
  assert(new Set(looks.map((look) => look.surface)).size >= 5);
  assert(new Set(looks.map((look) => JSON.stringify(look))).size >= 12);
});

test("the avatar seed can reach every face, surface, and ring style", () => {
  const looks = Array.from({ length: 512 }, (_, index) => orbitAppearance(`sprite-${index}`, "#83b6ab"));
  assert.deepEqual(new Set(looks.map(look => look.eyes)), new Set(eyeStyles));
  assert.deepEqual(new Set(looks.map(look => look.smile)), new Set(smileStyles));
  assert.deepEqual(new Set(looks.map(look => look.surface)), new Set(planetStyles));
  assert.deepEqual(new Set(looks.map(look => look.rings)), new Set(["none", "flat", "tilted", "double"]));
  assert(looks.every(look => Number.isInteger(look.variation) && look.variation >= 0 && look.variation < 8));
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
  for (const [before, after] of [[identity, first], [first, second]]) {
    const oldLook = orbitAppearance(before, color), newLook = orbitAppearance(after, color);
    assert(newLook.eyes !== oldLook.eyes || newLook.smile !== oldLook.smile, "face changes");
    assert(newLook.surface !== oldLook.surface || newLook.rings !== oldLook.rings, "planet changes");
  }
});

test("avatar regeneration retries a seed that would repeat the same face", () => {
  const identity = "same-seed";
  const generated = generateOrbitAvatarSeed(identity, "#83b6ab", () => identity);
  assert.notEqual(generated, identity);
  assert.notDeepEqual(orbitAppearance(generated, "#83b6ab"), orbitAppearance(identity, "#83b6ab"));
});
