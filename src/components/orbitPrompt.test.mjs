import test from "node:test";
import assert from "node:assert/strict";
import { localPromptReview, orbitPreamble } from "./orbitPrompt.ts";

const draft = { name: "Nova", role: "Research", personality: "Warm and precise", instructions: "Compare primary sources. Never purchase anything." };
test("prompt comparison preserves identity and original user restrictions", () => {
  const review = localPromptReview(draft);
  assert.equal(review.originalPrompt, orbitPreamble(draft));
  assert.ok(review.recommendedInstructions.startsWith(draft.instructions));
  assert.ok(review.recommendedPrompt.includes("Personality: Warm and precise"));
  assert.equal(review.source, "template");
});
test("regeneration uses the original draft rather than compounding recommendations", () => {
  const first = localPromptReview(draft, 0);
  const second = localPromptReview(draft, 1);
  assert.notEqual(first.recommendedInstructions, second.recommendedInstructions);
  assert.equal(second.recommendedInstructions.match(/Working approach:/g).length, 1);
  assert.equal(draft.instructions, "Compare primary sources. Never purchase anything.");
});
test("a long prompt is preserved rather than silently truncated or made unsavable", () => {
  const review = localPromptReview({ ...draft, instructions: "é".repeat(16_000) });
  assert.equal(new TextEncoder().encode(review.recommendedInstructions).length, 32_000);
  assert.ok(review.analysis[0].includes("preserved"));
});
