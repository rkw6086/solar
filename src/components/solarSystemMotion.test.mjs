import assert from "node:assert/strict";
import test from "node:test";
import { orbitPosition, orbitTracks, ORBIT_PERIOD } from "./solarSystemMotion.ts";

const roster = (count) => Array.from({ length: count }, (_, index) => ({ id: `orbit-${String(index).padStart(3, '0')}` }));

test("empty systems contain no invented Sprites", () => assert.deepEqual(orbitTracks([]), []));
test("tracks are deterministic regardless of input order", () => {
  const orbits = roster(18);
  assert.deepEqual(orbitTracks(orbits), orbitTracks([...orbits].reverse()));
});
test("eighteen Sprites share two resonant lanes with stable separation", () => {
  const tracks = orbitTracks(roster(18));
  assert.equal(tracks.filter(t => t.period === ORBIT_PERIOD).length, 6);
  assert.equal(tracks.filter(t => t.period === ORBIT_PERIOD * 2).length, 12);
  for (const elapsed of [0, 30000, 60000, 120000, 500000]) {
    const points = tracks.map(t => orbitPosition(t, elapsed, 840, 440));
    for (let i = 0; i < points.length; i++) {
      for (let j = 0; j < i; j++) {
        assert.ok(Math.hypot(points[i].x - points[j].x, points[i].y - points[j].y) > 44, `collision at ${elapsed}: ${i}, ${j}`);
      }
    }
  }
});
test("the whole system returns to its starting phase every outer orbit", () => {
  for (const track of orbitTracks(roster(18))) {
    const first = orbitPosition(track, 0, 700, 440);
    const repeated = orbitPosition(track, ORBIT_PERIOD * 2, 700, 440);
    assert.ok(Math.abs(first.x - repeated.x) < 1e-9);
    assert.ok(Math.abs(first.y - repeated.y) < 1e-9);
  }
});
test("positions remain bounded after resize", () => {
  for (const count of [1, 3, 18, 36]) {
    for (const track of orbitTracks(roster(count))) {
      for (const elapsed of [0, 12000, 45321, 250000]) {
        const { x, y } = orbitPosition(track, elapsed, 440, 320);
        assert.ok(Math.abs(x) <= 440 * .44);
        assert.ok(Math.abs(y) <= 320 * .44);
      }
    }
  }
});

test("twenty and forty Sprites use distinct, proportionate lanes with room for their smaller characters", () => {
  for (const count of [20, 40]) {
    const tracks = orbitTracks(roster(count));
    const minimum = count > 36 ? 32 : 36;
    const lanes = [...new Set(tracks.map(track => track.x))];
    assert.equal(lanes.length, count > 36 ? 4 : 3);
    assert.ok(lanes[lanes.length - 1] <= .4);
    for (const [width, height] of [[840, 440], [718, 320]]) {
    for (let elapsed = 0; elapsed < 480000; elapsed += 1000) {
      const points = tracks.map(track => orbitPosition(track, elapsed, width, height));
      for (let index = 0; index < points.length; index++) {
        assert.ok(Math.abs(points[index].x) <= width * .4);
        assert.ok(Math.abs(points[index].y) <= height * .4);
        for (let other = 0; other < index; other++) {
          assert.ok(Math.hypot(points[index].x - points[other].x, points[index].y - points[other].y) >= minimum - 1e-9);
        }
      }
    }
    }
  }
});
