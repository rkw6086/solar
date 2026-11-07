import assert from "node:assert/strict";
import test from "node:test";
import { BIRTH_TIMING, RIPPLE_COUNT, birthCuesBetween, rippleTiming, winkAmount } from "./birthTimeline.ts";

test("all eighteen ripple arrivals precede star formation", () => {
  const arrivals = Array.from({ length: RIPPLE_COUNT }, (_, index) => rippleTiming(index).arrival);
  assert.equal(arrivals.length, 18);
  assert.ok(arrivals.every((at, index) => at < BIRTH_TIMING.burst && (index === 0 || at > arrivals[index - 1])));
});

test("normal animation frames deliver every visual cue once", () => {
  let previous = 0;
  const received = [];
  for (let elapsed = 16; elapsed <= BIRTH_TIMING.opening; elapsed += 16) {
    received.push(...birthCuesBetween(previous, elapsed, false));
    previous = elapsed;
  }
  assert.deepEqual(received.filter(({ cue }) => cue === "ripple").map(({ index }) => index), Array.from({ length: 18 }, (_, index) => index));
  assert.equal(received.filter(({ cue }) => cue === "formation").length, 1);
  assert.equal(received.filter(({ cue }) => cue === "wink").length, 1);
});

test("a delayed frame never releases a backlog of ripple sounds", () => {
  const latest = rippleTiming(17).arrival;
  assert.deepEqual(birthCuesBetween(0, latest + 20, false).map(({ index }) => index), [17]);
  assert.deepEqual(birthCuesBetween(0, BIRTH_TIMING.opening, false), []);
  assert.deepEqual(birthCuesBetween(5_000, 5_000, false), []);
});

test("reduced motion has one short formation cue and no ripple or wink sequence", () => {
  let previous = 0;
  const received = [];
  for (let elapsed = 16; elapsed <= BIRTH_TIMING.reducedOpening; elapsed += 16) {
    received.push(...birthCuesBetween(previous, elapsed, true));
    previous = elapsed;
  }
  assert.deepEqual(received.map(({ cue }) => cue), ["formation"]);
});

test("wink closes and reopens before the opening finishes", () => {
  assert.equal(winkAmount(BIRTH_TIMING.wink - 1), 0);
  assert.equal(winkAmount(BIRTH_TIMING.wink + BIRTH_TIMING.winkDuration * 0.4), 1);
  assert.equal(winkAmount(BIRTH_TIMING.wink + BIRTH_TIMING.winkDuration), 0);
  assert.ok(BIRTH_TIMING.wink + BIRTH_TIMING.winkDuration < BIRTH_TIMING.opening);
});
