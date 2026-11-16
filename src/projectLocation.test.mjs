import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";

const source = await readFile(new URL("./projectLocation.ts", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { previewProjectLocation } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

test("preview names projects after their selected folder without claiming Git detection", () => {
  assert.deepEqual(previewProjectLocation(" /Users/sam/Project Folder/ "), { directory: "/Users/sam/Project Folder", name: "Project Folder", isGitRoot: false });
  assert.equal(previewProjectLocation("C:\\Work\\Solar\\").name, "Solar");
});

test("preview rejects unnamed roots and dot folders", () => {
  for (const directory of ["", " ", "/", ".", "..", "C:\\"]) {
    assert.throws(() => previewProjectLocation(directory));
  }
});
