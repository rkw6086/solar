import assert from "node:assert/strict";
import test from "node:test";
import { orbitPosition, orbitTracks } from "./solarSystemMotion.ts";

test("empty systems contain no invented Orbits", () => {
  assert.deepEqual(orbitTracks([]), []);
});

test("the same Orbits keep their tracks and phases regardless of input ordering", () => {
  const orbits = [{ id: "research" }, { id: "assistant" }, { id: "quality" }];
  assert.deepEqual(orbitTracks(orbits), orbitTracks([...orbits].reverse()));
  assert.deepEqual(orbits.map(({ id }) => id), ["research", "assistant", "quality"]);
});

test("integer periods return each Orbit to its starting point in resonance", () => {
  const tracks = orbitTracks([{ id: "a" }, { id: "b" }, { id: "c" }, { id: "d" }]);
  assert.deepEqual(tracks.map(({ period }) => period / tracks[0].period), [1, 2, 3, 4]);
  for (const track of tracks) {
    const first = orbitPosition(track, 0, 700, 400);
    const repeated = orbitPosition(track, 12 * tracks[0].period, 700, 400);
    assert.ok(Math.abs(first.x - repeated.x) < 1e-9);
    assert.ok(Math.abs(first.y - repeated.y) < 1e-9);
  }
});

test("shared-prefix IDs start spread around the star instead of in a pile", () => {
  for (const count of [2, 4, 8]) {
    const tracks = orbitTracks(Array.from({ length: count }, (_, index) => ({ id: `orbital-fixture-${index + 1}` })));
    const phases = tracks.map(({ phase }) => phase).sort((a, b) => a - b);
    const spacing = Math.PI * 2 / count;
    for (let index = 0; index < phases.length; index += 1) {
      const next = phases[(index + 1) % phases.length] + (index === phases.length - 1 ? Math.PI * 2 : 0);
      assert.ok(next - phases[index] >= spacing * 0.8 - 1e-12);
      assert.ok(next - phases[index] <= spacing * 1.2 + 1e-12);
    }
  }
});

test("all positions remain on their bounded ellipses after resize", () => {
  for (const count of [1, 3, 12]) {
    for (const track of orbitTracks(Array.from({ length: count }, (_, i) => ({ id: String(i) })))) {
      for (const [width, height] of [[440, 320], [700, 450]]) {
        for (const elapsed of [0, 12_000, 45_321, 250_000]) {
          const { x, y } = orbitPosition(track, elapsed, width, height);
          assert.ok(Math.abs(x) <= width * 0.4);
          assert.ok(Math.abs(y) <= height * 0.35);
          const ellipse = (x / (width * track.x)) ** 2 + (y / (height * track.y)) ** 2;
          assert.ok(Math.abs(ellipse - 1) < 1e-12);
        }
      }
    }
  }
});
