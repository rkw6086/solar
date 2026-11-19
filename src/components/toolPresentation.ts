import type { McpServer, Message, ToolActivity, ToolFileDiff } from "../types.ts";

export type ToolCategory = "shell" | "web" | "mcp" | "files" | "memory" | "team" | "routine" | "computer" | "generic";
export interface ToolPresentation {
  category: ToolCategory;
  title: string;
  summary: string;
  command?: string;
  directory?: string;
  output?: string;
  outcome?: string;
  failure?: string;
  status: ToolActivity["status"];
  truncated: boolean;
  diffs: ToolFileDiff[];
}

const record = (value: unknown): Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
const string = (value: unknown): string => typeof value === "string" ? value : "";
const compact = (value: unknown, limit = 180): string => cleanOutput(string(value)).replace(/\s+/g, " ").trim().slice(0, limit);
export function cleanOutput(value: string): string {
  // Console styling and cursor controls are not meaningful in a plain-text transcript.
  return value.replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "").replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "").replace(/\r\n?/g, "\n").replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, "");
}
function parse(value: string): unknown {
  try { return JSON.parse(value); } catch { return undefined; }
}
function sourceUrl(value: unknown): string {
  try {
    const url = new URL(string(value));
    if (url.protocol !== "http:" && url.protocol !== "https:") return "";
    return `${url.hostname}${url.port ? `:${url.port}` : ""}${url.pathname === "/" ? "" : url.pathname}`.slice(0, 220);
  } catch { return ""; }
}
function mcpName(name: string, servers: McpServer[]): { service: string; command: string } {
  const rest = name.slice("mcp__".length);
  const configured = servers.flatMap((server) => [server.id.replaceAll("-", "_"), server.id].map((alias) => ({ server, alias })))
    .sort((a, b) => b.alias.length - a.alias.length)
    .find(({ alias }) => rest.startsWith(`${alias}__`));
  const separator = rest.indexOf("__");
  return {
    service: configured?.server.name || (separator >= 0 ? rest.slice(0, separator).replaceAll("_", " ") : "Connected service"),
    command: configured ? rest.slice(configured.alias.length + 2) : separator >= 0 ? rest.slice(separator + 2) : rest,
  };
}
function routineSummary(args: Record<string, unknown>, result: Record<string, unknown>, fallback: string): string {
  const saved = Object.keys(record(result.routine)).length ? record(result.routine) : result;
  const value = (key: string) => Object.hasOwn(args, key) ? args[key] : saved[key];
  const prompt = compact(value("title"), 160) || compact(value("prompt"), 300);
  const trigger = value("trigger");
  const minutes = value("intervalMinutes");
  const interval = typeof minutes === "number" && Number.isFinite(minutes) && minutes > 0
    ? `${minutes} ${minutes === 1 ? "minute" : "minutes"}` : "";
  const timing = trigger === "cron" ? `Scheduled: ${compact(value("cron"), 100)} (local time)`
    : trigger === "folder" ? "When project files change"
    : trigger === "completion" ? "After a task completes"
    : trigger === "interval" ? interval ? `Every ${interval}` : "On a repeating interval"
    : interval ? `Repeat interval: ${interval}` : "";
  const enabled = value("enabled");
  const state = typeof enabled === "boolean" ? enabled ? "Enabled" : "Paused" : "";
  return [prompt, timing, state].filter(Boolean).join(" · ") || fallback;
}
function legacyActivity(message: Message): ToolActivity {
  const newline = message.content.indexOf("\n");
  const name = newline >= 0 ? message.content.slice(0, newline) : message.content;
  const output = newline >= 0 ? message.content.slice(newline + 1) : "";
  const data = record(parse(output));
  return {
    name: /^[\w.-]+$/.test(name) ? name : "tool",
    callId: message.id,
    arguments: {},
    output,
    status: data.success === false ? "failed" : "completed",
    truncated: data.truncated === true,
    diffs: [],
    exitCode: typeof data.exitCode === "number" ? data.exitCode : undefined,
  };
}
function outcome(status: ToolActivity["status"], exitCode?: number): string {
  if (status === "running") return "Running";
  if (status === "denied") return "Not allowed";
  if (status === "cancelled") return "Stopped";
  if (status === "interrupted") return "Interrupted";
  if (status === "failed") return exitCode !== undefined ? `Exit code ${exitCode}` : "Couldn’t finish";
  return exitCode !== undefined ? `Exit code ${exitCode}` : "Complete";
}

function failureSummary(activity: ToolActivity): string | undefined {
  if (activity.status === "denied") return "This action wasn’t allowed. Nothing was run.";
  if (activity.status === "cancelled") return "Stopped before this action finished.";
  if (activity.status === "interrupted") return "This action was interrupted. Its result has not been confirmed.";
  if (activity.status !== "failed") return undefined;
  const text = activity.output.toLowerCase();
  if (/timed? out|timeout/.test(text)) return "The tool took too long to respond. The Orbit can retry.";
  if (/not found|no such file/.test(text)) return "The requested file or resource couldn’t be found.";
  if (/permission|not allowed|denied/.test(text)) return "Access was denied. Check the Orbit’s access or permissions.";
  if (/unauthori[sz]ed|authentication|api key|401/.test(text)) return "The service needs a valid connection. Check its sign-in settings.";
  if (/connection|network|dns|unreachable/.test(text)) return "The service couldn’t be reached. Check the connection and try again.";
  return "This tool couldn’t finish. The Orbit can try another approach.";
}

/** Deliberately projects useful labels from tool data instead of dumping payloads. */
export function presentTool(message: Message, servers: McpServer[] = []): ToolPresentation {
  const activity = message.toolActivity ?? legacyActivity(message);
  const args = record(activity.arguments);
  const result = record(parse(activity.output));
  const name = activity.name;
  const base: ToolPresentation = {
    category: "generic", title: "Tool activity", summary: name.replaceAll("_", " "), status: activity.status,
    outcome: outcome(activity.status, activity.exitCode), truncated: activity.truncated || result.truncated === true,
    failure: failureSummary(activity),
    diffs: activity.diffs ?? [],
  };
  if (name === "shell_run") {
    const structured = !message.toolActivity && (typeof result.stdout === "string" || typeof result.stderr === "string");
    const stdout = string(result.stdout);
    const stderr = string(result.stderr);
    const output = structured ? `${stdout}${stdout && stderr && !stdout.endsWith("\n") ? "\n" : ""}${stderr ? `${stdout ? "\n" : ""}Standard error:\n${stderr}` : ""}` : activity.output;
    const code = activity.exitCode ?? (typeof result.exitCode === "number" ? result.exitCode : undefined);
    return { ...base, category: "shell", title: "Shell command", summary: compact(args.command, 160) || "Command output", command: cleanOutput(string(args.command)), directory: compact(args.workingDirectory, 240), output: cleanOutput(output), outcome: outcome(activity.status, code) };
  }
  if (name.startsWith("mcp__")) {
    const { service, command } = mcpName(name, servers);
    return { ...base, category: "mcp", title: service, summary: command || "Connected command" };
  }
  if (name === "web_fetch" || name.startsWith("browser_")) {
    const source = sourceUrl(args.url) || sourceUrl(result.url);
    const title = compact(result.title);
    const query = compact(args.query, 260);
    const titles: Record<string, string> = { web_fetch: "Read a web page", browser_open: "Open a web page", browser_search: "Search the web", browser_inspect: "Read the current page", browser_click: "Use a page control", browser_type: "Fill a page field", browser_screenshot: "Capture the browser" };
    return { ...base, category: "web", title: titles[name] || "Browser activity", summary: query ? `“${query}”` : [title, source].filter(Boolean).join(" · ") || (name === "browser_type" ? "Entering text in the selected field" : "Solar browser") };
  }
  if (name.startsWith("files_")) {
    const path = compact(args.path || result.path, 300) || "Orbit workspace";
    const titles: Record<string, string> = { files_read: "Read a file", files_write: "Write a file", files_list: "Browse files", files_search: "Search files" };
    const count = Array.isArray(result.matches) ? result.matches.length : Array.isArray(result.entries) ? result.entries.length : undefined;
    return { ...base, category: "files", title: titles[name] || "File activity", summary: args.query ? `“${compact(args.query)}” · ${path}` : path, outcome: count !== undefined && activity.status === "completed" ? `${count} ${name === "files_search" ? "matches" : "items"}` : base.outcome };
  }
  if (name === "pull_from_user_home") {
    const count = Array.isArray(result.copied) ? result.copied.length : Array.isArray(args.files) ? args.files.length : undefined;
    return { ...base, category: "files", title: "Import into Orbit home", summary: count === undefined ? "Copy approved home files and their dependencies" : `${count} ${count === 1 ? "file" : "files"} · private copies for this Orbit` };
  }
  if (name.startsWith("memory_")) return { ...base, category: "memory", title: name === "memory_save" ? "Save a memory" : "Search memories", summary: args.query ? `“${compact(args.query)}”` : args.shared ? "Shared project memory" : "Private Orbit memory" };
  if (name === "orbit_delegate") return { ...base, category: "team", title: "Ask a teammate", summary: compact(args.task, 240) || "Delegating a focused task" };
  if (name === "orbit_list") return { ...base, category: "team", title: "Find teammates", summary: Array.isArray(parse(activity.output)) ? `${(parse(activity.output) as unknown[]).length} Orbits available` : "Check the Solar team" };
  if (name === "routine_list") {
    const parsed = parse(activity.output);
    const routines = Array.isArray(parsed) ? parsed : Array.isArray(result.routines) ? result.routines : undefined;
    const count = routines?.length;
    const enabled = routines?.filter((routine) => record(routine).enabled === true).length;
    return { ...base, category: "routine", title: "View routines", summary: count !== undefined && activity.status === "completed" ? `${count} ${count === 1 ? "routine" : "routines"} · ${enabled} enabled` : "View this Orbit’s saved routines" };
  }
  const routineTitles: Record<string, [string, string]> = {
    routine_create: ["Create routine", "Set up a recurring task"],
    routine_update: ["Update routine", "Change this routine"],
    routine_delete: ["Delete routine", "Remove this saved routine"],
    routine_run: ["Run routine", "Start this routine now"],
  };
  if (routineTitles[name]) {
    const [title, fallback] = routineTitles[name];
    const queued = name === "routine_run" && activity.status === "completed" && result.status === "queued";
    return { ...base, category: "routine", title, summary: routineSummary(args, result, queued ? "Queued in a separate routine conversation" : fallback), outcome: queued ? "Queued" : base.outcome };
  }
  if (name === "project_list") return { ...base, category: "files", title: "Find project folders", summary: "Check available projects and workspaces" };
  if (name.startsWith("computer_") || name === "screen_capture" || name === "native_permissions") {
    const titles: Record<string, string> = { computer_click: "Click a screen control", computer_type: "Enter text", computer_key: "Press a key", computer_inspect: "Inspect the screen", screen_capture: "Capture the shared screen", native_permissions: "Check desktop access" };
    return { ...base, category: "computer", title: titles[name] || "Desktop activity", summary: name === "computer_key" ? compact(args.key) || "Keyboard input" : "Shared desktop" };
  }
  return base;
}

export function presentApproval(name: string, args: unknown, servers: McpServer[] = []): ToolPresentation {
  return presentTool({ id: "approval", conversationId: "", role: "tool", content: "", createdAt: "", toolActivity: { callId: "approval", name, arguments: args, output: "", status: "running", truncated: false, diffs: [] } }, servers);
}

export function diffLineKind(line: string): "add" | "remove" | "hunk" | "header" | "context" {
  if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff ") || line.startsWith("index ")) return "header";
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "remove";
  return "context";
}
export function diffCounts(patch: string): { added: number; removed: number } {
  return patch.split("\n").reduce((count, line) => {
    const kind = diffLineKind(line);
    if (kind === "add") count.added++;
    if (kind === "remove") count.removed++;
    return count;
  }, { added: 0, removed: 0 });
}
