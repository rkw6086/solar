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
  assert.equal(presentTool(message("mcp__svc__call", {}, '{"private":"secret"}', "failed")).failure, "This tool couldn’t finish. The Sprite can try another approach.");
});

test("approvals retain exact shell command and useful context without JSON", () => {
  const view = presentApproval("shell_run", { command: "printf 'hello' && cargo test", workingDirectory: "/code" });
  assert.equal(view.command, "printf 'hello' && cargo test");
  assert.equal(view.directory, "/code");
});

test("routine creation shows the task, timing, and enabled state without raw data", () => {
  const view = presentTool(message("routine_create", { prompt: "Review open issues", trigger: "interval", intervalMinutes: 30, enabled: true, projectId: "private-project-id" }, '{"internal":"do not display"}', "running"));
  assert.equal(view.category, "routine");
  assert.equal(view.title, "Create routine");
  assert.equal(view.summary, "Review open issues · Every 30 minutes · Enabled");
  assert.equal(view.outcome, "Running");
  assert.equal(view.output, undefined);
  assert(!JSON.stringify(view).includes("private-project-id"));
  assert(!JSON.stringify(view).includes("do not display"));
});

test("routine updates preserve false and summarize event triggers", () => {
  const paused = presentTool(message("routine_update", { routineId: "private-id", enabled: false }, JSON.stringify({ prompt: "Check the build", trigger: "interval", intervalMinutes: 1, enabled: true })));
  assert.equal(paused.title, "Update routine");
  assert.equal(paused.summary, "Check the build · Every 1 minute · Paused");
  assert(!JSON.stringify(paused).includes("private-id"));
  for (const [trigger, timing] of [["folder", "When project files change"], ["completion", "After a task completes"]]) {
    const view = presentTool(message("routine_update", { trigger, intervalMinutes: 60 }, ""));
    assert.equal(view.summary, timing);
  }
  assert.equal(presentTool(message("routine_update", { intervalMinutes: 15 }, "")).summary, "Repeat interval: 15 minutes");
});

test("routine summaries read saved values and do not invent missing schedules", () => {
  const saved = { routine: { prompt: "Prepare my daily brief", trigger: "interval", intervalMinutes: 60, enabled: true }, secret: "hidden" };
  assert.equal(presentTool(message("routine_create", { prompt: "Prepare my daily brief" }, JSON.stringify(saved))).summary, "Prepare my daily brief · Every 60 minutes · Enabled");
  assert.equal(presentTool(message("routine_create", { prompt: "Prepare my daily brief" }, "", "running")).summary, "Prepare my daily brief");
  assert.equal(presentTool(message("routine_create", { trigger: "interval", intervalMinutes: 0 }, "", "failed")).summary, "On a repeating interval");
});

test("routine lists report counts without exposing stored prompts or identifiers", () => {
  const routines = [{ id: "private-id", prompt: "private task", enabled: true }, { enabled: false }];
  for (const payload of [routines, { routines }]) {
    const view = presentTool(message("routine_list", {}, JSON.stringify(payload)));
    assert.equal(view.category, "routine");
    assert.equal(view.title, "View routines");
    assert.equal(view.summary, "2 routines · 1 enabled");
    assert(!JSON.stringify(view).includes("private"));
  }
  assert.equal(presentTool(message("routine_list", {}, "[]")).summary, "0 routines · 0 enabled");
  assert.equal(presentTool(message("routine_list", {}, '[{"enabled":true}]')).summary, "1 routine · 1 enabled");
  assert.equal(presentTool(message("routine_list", {}, "", "running")).summary, "View this Sprite’s saved routines");
});

test("routine run and delete cards use friendly action labels", () => {
  for (const [name, title, summary] of [["routine_run", "Run routine", "Start this routine now"], ["routine_delete", "Delete routine", "Remove this saved routine"]]) {
    const view = presentTool(message(name, { routineId: "private-id" }, '{"taskId":"hidden-id"}'));
    assert.equal(view.category, "routine");
    assert.equal(view.title, title);
    assert.equal(view.summary, summary);
    assert.equal(view.output, undefined);
    assert(!JSON.stringify(view).includes("private-id"));
    assert(!JSON.stringify(view).includes("hidden-id"));
  }
});

test("starting a routine reports queued work rather than a finished routine", () => {
  const payload = JSON.stringify({ routineId: "private-id", conversationId: "hidden-id", status: "queued" });
  const queued = presentTool(message("routine_run", { routineId: "private-id" }, payload));
  assert.equal(queued.outcome, "Queued");
  assert.equal(queued.summary, "Queued in a separate routine conversation");
  assert.equal(queued.output, undefined);
  assert(!JSON.stringify(queued).includes("private-id"));
  assert(!JSON.stringify(queued).includes("hidden-id"));
  assert.equal(presentTool(message("routine_run", {}, payload, "running")).outcome, "Running");
  assert.equal(presentTool(message("routine_run", {}, payload, "failed")).outcome, "Couldn’t finish");
  assert.equal(presentTool(message("routine_run", {}, "{}")).outcome, "Complete");
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

test("workspace link requests explain consent and do not display file payloads", () => {
  const request = presentApproval("pull_from_user_home", { links: [{ source: "~/.kube", destination: "~/.kube", directory: true }], notice: "Approve this link" });
  assert.equal(request.title, "Link items into the workspace");
  assert.equal(request.summary, "1 item · requires your approval");
  const result = presentTool(message("pull_from_user_home", {}, JSON.stringify({ linked: [{ source: "~/.config", destination: "~/.config" }], private: "do not render" })));
  assert.equal(result.summary, "1 item · requires your approval");
  assert(!JSON.stringify(result).includes("do not render"));
  const legacy = presentTool(message("pull_from_user_home", {}, JSON.stringify({ copied: ["old-file"] })));
  assert.equal(legacy.summary, "1 file · previously imported copies");
});

test("home discovery shows the search and count without private paths or contents", () => {
  const view = presentTool(message("search_user_home", { query: "kube" }, JSON.stringify({ matches: [{ path: "/private/home/.kube" }], private: "hidden" })));
  assert.equal(view.title, "Find files to request");
  assert.equal(view.summary, "Names matching “kube” · home directory");
  assert.equal(view.outcome, "1 match");
  assert(!JSON.stringify(view).includes("/private/home"));
});
