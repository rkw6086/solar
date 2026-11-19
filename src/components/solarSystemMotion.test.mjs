import assert from "node:assert/strict";
import test from "node:test";
import { orbitPosition, orbitTracks, ORBIT_PERIOD } from "./solarSystemMotion.ts";

const roster = (count) => Array.from({ length: count }, (_, index) => ({ id: `orbit-${String(index).padStart(3, '0')}` }));

test("empty systems contain no invented Sprites", () => assert.deepEqual(orbitTracks([]), []));
test("tracks are deterministic regardless of input order without changing the roster", () => {
  const orbits = roster(18).reverse();
  const original = [...orbits];
  assert.deepEqual(orbitTracks(orbits), orbitTracks([...orbits].reverse()));
  assert.deepEqual(orbits, original);
});
test("every Sprite owns a distinct concentric path", () => {
  for (const count of [1, 3, 6, 18, 40, 64]) {
    const tracks = orbitTracks(roster(count));
    assert.equal(tracks.length, count);
    assert.equal(new Set(tracks.map(track => track.x)).size, count);
    assert.ok(tracks.every(track => track.x === track.y));
    assert.ok(tracks.every(track => track.x >= .22 && track.x <= .42 + 1e-12));
    for (let index = 1; index < count; index++) assert.ok(tracks[index].x > tracks[index - 1].x);
  }
});
test("a complete 1:1 resonance returns all Sprites to their starting positions", () => {
  for (const track of orbitTracks(roster(40))) {
    assert.equal(track.period, ORBIT_PERIOD);
    const first = orbitPosition(track, 0, 700, 440);
    const repeated = orbitPosition(track, ORBIT_PERIOD, 700, 440);
    assert.ok(Math.abs(first.x - repeated.x) < 1e-9);
    assert.ok(Math.abs(first.y - repeated.y) < 1e-9);
  }
});
test("distinct paths stay inside the stage after resize throughout a full orbit", () => {
  for (const count of [1, 3, 6, 18, 40]) {
    for (const track of orbitTracks(roster(count))) {
      for (let elapsed = 0; elapsed <= ORBIT_PERIOD; elapsed += 1000) {
        const { x, y } = orbitPosition(track, elapsed, 440, 320);
        assert.ok(Math.abs(x) <= 440 * .42 + 1e-9);
        assert.ok(Math.abs(y) <= 320 * .42 + 1e-9);
      }
    }
  }
});
test("hover-enlarged characters remain separated in compact and wide scenes", () => {
  for (const count of [3, 6, 18, 20, 36, 40]) {
    const tracks = orbitTracks(roster(count));
    // The pixel body occupies 75% of its SVG; leave room for 10% hover growth.
    const artworkSize = count > 36 ? 32 : count > 18 ? 36 : count > 8 ? 42 : 58;
    const minimum = artworkSize * .75 * 1.1 + 2;
    for (const [width, height] of [[840, 440], [718, 320]]) {
      for (let elapsed = 0; elapsed <= ORBIT_PERIOD; elapsed += 1000) {
        const points = tracks.map(track => orbitPosition(track, elapsed, width, height));
        for (let index = 0; index < points.length; index++) {
          for (let other = 0; other < index; other++) {
            assert.ok(Math.hypot(points[index].x - points[other].x, points[index].y - points[other].y) >= minimum,
              `collision for ${count} Sprites at ${elapsed}: ${index}, ${other}`);
          }
        }
      }
    }
  }
});
