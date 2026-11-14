import test from "node:test";
import assert from "node:assert/strict";
import { cleanOutput, diffCounts, diffLineKind, presentApproval, presentTool } from "./toolPresentation.ts";

function message(name, args, output, status = "completed", extra = {}) {
  return { id: "m", conversationId: "c", createdAt: "", role: "tool", content: "", toolActivity: { callId: "call", name, arguments: args, output, status, truncated: false, diffs: [], ...extra } };
}

test("shell shows command and incremental output without JSON wrappers", () => {
  const running = presentTool(message("shell_run", { command: "cargo test", workingDirectory: "/code/solar" }, "Compiling solar…\n", "running"));
  assert.equal(running.command, "cargo test");
  assert.equal(running.directory, "/code/solar");
  assert.equal(running.output, "Compiling solar…\n");
  assert.equal(running.outcome, "Running");
  const failed = presentTool(message("shell_run", { command: "cargo test" }, "test failed\n", "failed", { exitCode: 1 }));
  assert.equal(failed.outcome, "Exit code 1");
  const rawJson = '{"stdout":"a program may print JSON"}';
  assert.equal(presentTool(message("shell_run", {}, rawJson)).output, rawJson);
});

test("legacy shell results remain readable without exposing their JSON", () => {
  const view = presentTool({ id: "m", role: "tool", content: 'shell_run\n{"stdout":"hello\\n","stderr":"warning\\n","exitCode":0,"success":true,"truncated":true}' });
  assert.equal(view.title, "Shell command");
  assert.equal(view.output, "hello\n\nStandard error:\nwarning\n");
  assert.equal(view.outcome, "Exit code 0");
  assert.equal(view.truncated, true);
  assert(!view.output.includes('"stdout"'));
});

test("web cards summarize searches and sources and never display fetched data", () => {
  const result = JSON.stringify({ title: "Planet guide", url: "https://example.org/planets?token=private", text: "DO_NOT_DISPLAY_HTML" });
  const search = presentTool(message("browser_search", { query: "Europa ocean evidence" }, result));
  assert.equal(search.summary, "“Europa ocean evidence”");
  assert.equal(search.output, undefined);
  const fetch = presentTool(message("web_fetch", { url: "https://example.org/planets?token=private" }, result));
  assert.equal(fetch.summary, "Planet guide · example.org/planets");
  assert(!JSON.stringify(fetch).includes("DO_NOT_DISPLAY_HTML"));
  assert(!fetch.summary.includes("private"));
});

test("MCP cards show service and command while omitting opaque arguments and results", () => {
  const view = presentTool(message("mcp__work_tools__search_contacts", { secret: "sensitive" }, '{"payload":"private"}'), [{ id: "work-tools", name: "Work directory" }]);
  assert.equal(view.title, "Work directory");
  assert.equal(view.summary, "search_contacts");
  assert(!JSON.stringify(view).includes("private"));
  assert(!JSON.stringify(view).includes("sensitive"));
  const uuid = "55d10944-82c8-4abe-8c28-8c50783a27ea";
  for (const alias of [uuid, uuid.replaceAll("-", "_")]) {
    assert.equal(presentTool(message(`mcp__${alias}__lookup`, {}, ""), [{ id: uuid, name: "My service" }]).title, "My service");
  }
});

test("file and generic tools use concise labels, not raw payloads", () => {
  const file = presentTool(message("files_write", { path: "/code/src/main.rs", content: "do not display entire file" }, '{"bytes":10,"written":true}'));
  assert.equal(file.summary, "/code/src/main.rs");
  assert.equal(file.title, "Write a file");
  assert.equal(file.output, undefined);
  const unknown = presentTool(message("custom_analysis", { secret: "private" }, '{"huge":"data"}'));
  assert.equal(unknown.summary, "custom analysis");
  assert.equal(unknown.output, undefined);
  assert.equal(presentTool(message("web_fetch", {}, "Network connection refused", "failed")).failure, "The service couldn’t be reached. Check the connection and try again.");
  assert.equal(presentTool(message("mcp__svc__call", {}, '{"private":"secret"}', "failed")).failure, "This tool couldn’t finish. The Orbit can try another approach.");
});

test("approvals retain exact shell command and useful context without JSON", () => {
  const view = presentApproval("shell_run", { command: "printf 'hello' && cargo test", workingDirectory: "/code" });
  assert.equal(view.command, "printf 'hello' && cargo test");
  assert.equal(view.directory, "/code");
});

test("diff headings are not counted as added and removed content", () => {
  assert.equal(diffLineKind("+++ b/a.rs"), "header");
  assert.equal(diffLineKind("@@ -1 +1 @@"), "hunk");
  assert.deepEqual(diffCounts("--- a/a.rs\n+++ b/a.rs\n@@ -1 +2 @@\n-old\n+new\n+extra\n context"), { added: 2, removed: 1 });
});

test("terminal escape codes cannot disrupt a text output pane", () => {
  assert.equal(cleanOutput("\u001b[31mred\u001b[0m\u0000\nnext"), "red\nnext");
  assert.equal(cleanOutput("\u001b]0;window title\u0007text"), "text");
  assert.equal(cleanOutput("step 1\rstep 2\r\n"), "step 1\nstep 2\n");
});
