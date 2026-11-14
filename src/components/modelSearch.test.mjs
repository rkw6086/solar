import test from "node:test";
import assert from "node:assert/strict";
import { filterModels, modelOptions } from "./modelSearch.ts";

const providers = [
  { id: "local", name: "Local rig", models: ["qwen-3", "same-model", " qwen-3 ", ""] },
  { id: "remote", name: "Work gateway", models: ["same-model", "claude-sonnet-4", "gpt-5.1", "literal[a]+b"] },
];
const options = modelOptions(providers);

test("catalog labels every model with its provider and retains provider-specific duplicates", () => {
  assert.equal(options.length, 6);
  assert.deepEqual(options.filter((item) => item.model === "same-model").map((item) => item.label), ["same-model (Local rig)", "same-model (Work gateway)"]);
  assert.deepEqual(options.map(({ model, providerName, label }) => label === `${model} (${providerName})`), options.map(() => true));
});

test("ordinary tokens match case-insensitive model or provider substrings in any order", () => {
  assert.deepEqual(filterModels(options, "gateway SONNET").map((item) => item.model), ["claude-sonnet-4"]);
  assert.equal(filterModels(options, "  LOCAL  ").length, 2);
  assert.equal(filterModels(options, "").length, options.length);
});

test("wildcards match across full labels while regex punctuation remains literal", () => {
  assert.deepEqual(filterModels(options, "cl*4 work*").map((item) => item.model), ["claude-sonnet-4"]);
  assert.deepEqual(filterModels(options, "gpt-?.1").map((item) => item.model), ["gpt-5.1"]);
  assert.deepEqual(filterModels(options, "[a]+b").map((item) => item.model), ["literal[a]+b"]);
  assert.equal(filterModels(options, "*").length, options.length);
  assert.equal(filterModels(options, "(local rig)").length, 2);
  assert.equal(filterModels(options, "does-not-exist").length, 0);
});

test("editing keeps a saved custom model only under its actual configured provider", () => {
  const current = modelOptions(providers, { providerId: "local", model: "custom-7" });
  assert.equal(current.filter((item) => item.model === "custom-7").length, 1);
  assert.equal(current.find((item) => item.model === "custom-7").label, "custom-7 (Local rig)");
  assert.equal(modelOptions(providers, { providerId: "missing", model: "custom-7" }).length, options.length);
  assert.equal(modelOptions(providers, { providerId: "local", model: "qwen-3" }).length, options.length);
});
