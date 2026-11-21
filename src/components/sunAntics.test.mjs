import assert from "node:assert/strict";
import test from "node:test";
import { nextSunAntic, sunAntics, sunAnticDelay, sunAnticDuration } from "./sunAntics.ts";

test("quiet visits vary instead of repeating the last greeting", () => {
  for (const previous of [null, ...sunAntics]) {
    const choices = Array.from({ length: 100 }, (_, index) => nextSunAntic(previous, () => index / 100));
    assert(choices.every(choice => sunAntics.includes(choice) && choice !== previous));
    assert.equal(new Set(choices).size, sunAntics.length - (previous === null ? 0 : 1));
  }
});

test("visits leave a quiet gap longer than every animation", () => {
  assert(sunAnticDelay(() => 0) >= 18000);
  assert(sunAnticDelay(() => 1) <= 32000);
  for (const antic of sunAntics) assert(sunAnticDuration(antic) < sunAnticDelay(() => 0));
});
