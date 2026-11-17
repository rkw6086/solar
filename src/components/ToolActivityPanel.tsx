import { useEffect, useRef, useState } from "react";
import { ArrowDown, Check, ChevronDown, CircleStop, FileCode2, Files, Globe2, LoaderCircle, Network, Repeat2, Terminal, Users, Wrench, X } from "lucide-react";
import type { ComponentType } from "react";
import type { McpServer, Message, ToolFileDiff } from "../types";
import { diffCounts, diffLineKind, presentApproval, presentTool } from "./toolPresentation";
import type { ToolCategory } from "./toolPresentation";
import "./toolActivity.css";

const icons: Record<ToolCategory, ComponentType<{ size?: number; className?: string }>> = {
  shell: Terminal, web: Globe2, mcp: Network, files: Files, memory: FileCode2, team: Users, routine: Repeat2, computer: Wrench, generic: Wrench,
};

function FileDiff({ diff }: { diff: ToolFileDiff }) {
  const count = diffCounts(diff.patch);
  const lines = diff.patch.split("\n");
  const shown = lines.slice(0, 1800);
  return <details className="tool-file-diff" open>
    <summary><FileCode2 size={14} /><span>{diff.path}</span><span className="diff-counts"><b>+{count.added}</b><i>−{count.removed}</i></span><ChevronDown size={13} /></summary>
    <pre aria-label={`Changes in ${diff.path}`} tabIndex={0}>{shown.map((line, index) => <span className={`diff-line diff-${diffLineKind(line)}`} key={index}>{line || " "}{"\n"}</span>)}</pre>
    {(diff.truncated || lines.length > shown.length) && <p className="tool-note">Preview shortened. Open the project to review the complete changes.</p>}
  </details>;
}

export function ToolActivityPanel({ message, servers = [] }: { message: Message; servers?: McpServer[] }) {
  const view = presentTool(message, servers);
  const running = view.status === "running";
  const [open, setOpen] = useState(view.category === "shell" || view.diffs.length > 0);
  const [following, setFollowing] = useState(true);
  const output = useRef<HTMLPreElement>(null);
  const follow = useRef(true);
  const Icon = icons[view.category];
  const hasOutput = view.category === "shell";
  const failed = view.status === "failed" || view.status === "denied";
  useEffect(() => {
    if (open && follow.current && output.current) output.current.scrollTop = output.current.scrollHeight;
  }, [view.output, open]);
  function followOutput() {
    follow.current = true;
    setFollowing(true);
    if (output.current) output.current.scrollTop = output.current.scrollHeight;
  }
  return <details className={`tool-activity tool-${view.category} tool-status-${view.status}`} open={open}
    onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary>
      <span className="tool-kind-icon"><Icon size={16} /></span>
      <span className="tool-heading"><strong>{view.title}{view.diffs.length > 0 && <em className="tool-file-count">{view.diffs.length} {view.diffs.length === 1 ? "file changed" : "files changed"}</em>}</strong><span>{view.summary}</span></span>
      <span className={`tool-state ${failed ? "tool-state-error" : ""}`} aria-label={view.outcome}>
        {running ? <LoaderCircle size={13} className="spin" /> : failed ? <X size={13} /> : ["cancelled", "interrupted"].includes(view.status) ? <CircleStop size={13} /> : <Check size={13} />}
        <span>{view.outcome}</span>
      </span>
      <ChevronDown className="tool-chevron" size={14} />
    </summary>
    <div className="tool-activity-body">
      {view.command && <pre className="tool-command" aria-label="Shell command"><code>{view.command}</code></pre>}
      {view.directory && <p className="tool-directory"><span>Working folder</span>{view.directory}</p>}
      {hasOutput ? <div className="tool-output-region">
        <div className="tool-output-label"><span>Output</span>{running && <span role="status">Receiving output…</span>}</div>
        <pre className="tool-output" ref={output} tabIndex={0} aria-label="Command output" aria-live="off"
          onScroll={(event) => {
            const pane = event.currentTarget;
            follow.current = pane.scrollHeight - pane.scrollTop - pane.clientHeight < 28;
            setFollowing(follow.current);
          }}>{view.output || (running ? "Waiting for output…" : "No output.")}</pre>
        {!following && running && <button className="tool-follow" type="button" onClick={followOutput}><ArrowDown size={12} />Follow output</button>}
      </div> : <p className="tool-detail-summary">{view.summary}</p>}
      {view.failure && <p className="tool-failure">{view.failure}</p>}
      {view.truncated && <p className="tool-note">Output was shortened to keep this conversation responsive.</p>}
      {view.diffs.length > 0 && <section className="tool-diffs" aria-label="File changes"><h4>File changes <span>observed during this tool</span></h4>{view.diffs.map((diff, index) => <FileDiff key={`${diff.path}-${index}`} diff={diff} />)}</section>}
    </div>
  </details>;
}

export function ToolApprovalDetails({ name, args, servers = [] }: { name: string; args: unknown; servers?: McpServer[] }) {
  const view = presentApproval(name, args, servers);
  return <div className="tool-approval-details">
    <p><strong>{view.title}</strong>{view.category === "shell" ? "" : <span>{view.summary}</span>}</p>
    {view.command && <pre className="tool-command" aria-label="Command awaiting approval"><code>{view.command}</code></pre>}
    {view.directory && <p className="tool-directory"><span>Working folder</span>{view.directory}</p>}
  </div>;
}
