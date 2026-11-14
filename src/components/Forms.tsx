import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import {
  LoaderCircle,
  ShieldCheck,
  Sparkles,
} from "lucide-react";
import { api, native } from "../api";
import { errorText } from "../types";
import type {
  Bootstrap,
  Conversation,
  McpServer,
  Memory,
  Orbit,
  Project,
  Provider,
  ProviderKind,
  Routine,
  Settings,
} from "../types";
import { Modal } from "./Modal";
import { Planet } from "./Planet";
import { ModelPicker } from "./ModelPicker";
import "./simpleOrbit.css";

const colors = [
  "#e8aa62",
  "#83b6ab",
  "#a49acc",
  "#dc8b8b",
  "#7eadd3",
  "#abc37c",
  "#ddba80",
  "#ce97bb",
];
const id = () => crypto.randomUUID();

function useSubmit(onSave: () => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function submit(action: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await action();
      onSave();
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setBusy(false);
    }
  }
  return { busy, error, submit };
}

function FormFooter({
  busy,
  error,
  label = "Save",
  onClose,
  children,
}: {
  busy: boolean;
  error: string;
  label?: string;
  onClose: () => void;
  children?: ReactNode;
}) {
  return (
    <>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <footer className="form-footer">
        <button className="button button-quiet" type="button" onClick={onClose}>
          Cancel
        </button>
        <div className="form-footer-right">
          {children}
          <button
            className="button button-primary"
            disabled={busy}
            type="submit"
          >
            {busy && <LoaderCircle size={15} className="spin" />}
            {label}
          </button>
        </div>
      </footer>
    </>
  );
}

export function OrbitForm({
  data,
  existing,
  onClose,
  onSaved,
}: {
  data: Bootstrap;
  existing?: Orbit;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [orbit, setOrbit] = useState<Orbit>(() => existing ? {
    ...existing, tasks: existing.tasks ?? "",
  } : {
    id: id(), name: "", gender: "", role: "Personal Orbit", personality: "",
    tasks: "", instructions: "", color: colors[data.orbits.length % colors.length],
    model: "", providerId: "", autonomy: "ask", toolGrants: ["*"],
    projectIds: [], enabled: true,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const request = useRef<string | null>(null);
  const mounted = useRef(true);
  const cancelSave = useCallback(() => {
    const requestId = request.current;
    request.current = null;
    if (requestId) void api("cancel_orbit_prompt_analysis", { requestId }).catch(() => {});
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; cancelSave(); };
  }, [cancelSave]);
  const close = useCallback(() => {
    cancelSave();
    onClose();
  }, [cancelSave, onClose]);
  function update<K extends keyof Orbit>(key: K, value: Orbit[K]) {
    setOrbit((current) => ({ ...current, [key]: value }));
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    if (!orbit.providerId || !orbit.model) {
      setError("Choose a model for your Orbit.");
      return;
    }
    setBusy(true);
    setError("");
    const requestId = id();
    request.current = requestId;
    try {
      const draft = { ...orbit, name: orbit.name.trim(), gender: (orbit.gender ?? "").trim(),
        tasks: (orbit.tasks ?? "").trim(), toolGrants: ["*"] };
      await api(existing ? "update_orbit_profile" : "create_orbit", { orbit: draft, requestId });
      if (mounted.current && request.current === requestId) onSaved();
    } catch (failure) {
      if (mounted.current && request.current === requestId) setError(errorText(failure));
    } finally {
      if (mounted.current && request.current === requestId) {
        request.current = null;
        setBusy(false);
      }
    }
  }
  return (
    <Modal title={existing ? `${existing.name}’s profile` : "Create your Orbit"}
      subtitle={existing ? "Shape their personality, tasks, and model." : "Tell us who they are and what you’d like their help with."}
      onClose={close} wide>
      <form onSubmit={(event) => void submit(event)} className="simple-orbit-form">
        <div className="simple-orbit-heading">
          <Planet name={orbit.name || "Your Orbit"} color={orbit.color} size={64} />
          <div><strong>{orbit.name || "Your next teammate"}</strong><p>{existing ? "All tools available" : "Solar builds their instructions automatically."}</p></div>
        </div>
        <div className="form-body">
          <div className="field-grid">
            <label className="field"><span>Name</span><input required maxLength={100} disabled={busy}
              autoComplete="off" placeholder="e.g. Nova" value={orbit.name} onChange={(event) => update("name", event.target.value)} /></label>
            <label className="field"><span>Gender <span className="optional">optional</span></span><input maxLength={200} disabled={busy}
              placeholder="e.g. female, male, nonbinary, or none" value={orbit.gender ?? ""} onChange={(event) => update("gender", event.target.value)} /></label>
          </div>
          <label className="field"><span>Personality</span><textarea required rows={2} maxLength={8000} disabled={busy}
            placeholder="Warm, curious, direct, with a dry sense of humor…" value={orbit.personality} onChange={(event) => update("personality", event.target.value)} /></label>
          <label className="field"><span>Tasks</span><textarea required rows={3} maxLength={32000} disabled={busy}
            placeholder="What should they help you do? Include any important boundaries or preferences."
            value={orbit.tasks ?? ""} onChange={(event) => update("tasks", event.target.value)} /></label>
          <ModelPicker providers={data.providers} providerId={orbit.providerId} model={orbit.model}
            required disabled={busy} onChange={(selection) => setOrbit((current) => ({ ...current, ...selection }))} />
          {!data.providers.some((provider) => provider.models.length) && (
            <p className="simple-orbit-note">Add a connection and discover its models in Connections before {existing ? "updating" : "creating"} an Orbit.</p>
          )}
          {existing ? <>
            <label className="field"><span>Role</span><input required maxLength={300} disabled={busy}
              value={orbit.role} onChange={(event) => update("role", event.target.value)} /></label>
            <label className="field"><span>When should they ask permission?</span>
              <select value={orbit.autonomy} disabled={busy} onChange={(event) => update("autonomy", event.target.value as Orbit["autonomy"])}>
                <option value="ask">Ask before every tool action</option>
                <option value="workspace">Ask before shell, browser, and connected tools</option>
                <option value="trusted">Use available tools without asking</option>
              </select></label>
            {data.projects.length > 0 && <fieldset className="project-checks"><legend>Project access</legend>
              {data.projects.map((project) => <label className="check-row" key={project.id}><input type="checkbox" disabled={busy}
                checked={orbit.projectIds.includes(project.id)} onChange={(event) => update("projectIds", event.target.checked
                  ? [...orbit.projectIds, project.id] : orbit.projectIds.filter((value) => value !== project.id))} /><span>{project.name}</span></label>)}
            </fieldset>}
          </> : <p className="simple-orbit-note">Your selected model will turn these details into working instructions. Your Orbit starts with all tools available and asks before acting. You can change its settings any time.</p>}
          {busy && <p className="simple-orbit-progress" role="status"><LoaderCircle size={16} className="spin" />{existing ? "Updating Orbit…" : "Creating Orbit…"}</p>}
        </div>
        <FormFooter busy={busy} error={error} onClose={close} label={existing ? "Save Orbit" : "Create Orbit"} />
      </form>
    </Modal>
  );
}

export function ProviderForm({
  existing,
  settings,
  onClose,
  onSaved,
}: {
  existing?: Provider;
  settings: Settings;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [provider, setProvider] = useState<Provider>(
    existing ?? {
      id: id(),
      name: "",
      kind: "openai",
      baseUrl: "",
      models: [],
      connected: false,
      experimental: false,
    },
  );
  const [apiKey, setApiKey] = useState("");
  const [modelText, setModelText] = useState(existing?.models.join("\n") ?? "");
  const [accepted, setAccepted] = useState(existing?.experimental ?? false);
  const status = useSubmit(onSaved);
  const subscription =
    provider.kind === "chatgpt" || provider.kind === "claude";
  const names: Record<ProviderKind, string> = {
    openai: "OpenAI compatible",
    litellm: "LiteLLM",
    chatgpt: "ChatGPT subscription",
    claude: "Claude subscription",
  };
  return (
    <Modal
      title={existing ? "Edit connection" : "Connect a model"}
      subtitle="Choose where your Orbits think. Their work stays organized here."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void status.submit(async () => {
            if (
              provider.kind === "claude" &&
              accepted &&
              !settings.experimentalClaude
            )
              await api("save_settings", {
                settings: { ...settings, experimentalClaude: true },
              });
            return api("save_provider", {
              provider: {
                ...provider,
                name: provider.name.trim() || names[provider.kind],
                models: modelText
                  .split(/\n|,/)
                  .map((model) => model.trim())
                  .filter(Boolean),
                experimental: provider.kind === "claude" && accepted,
                apiKey: apiKey || undefined,
              },
            });
          });
        }}
      >
        <div className="form-body">
          <label className="field">
            <span>Connection type</span>
            <select
              value={provider.kind}
              onChange={(event) =>
                setProvider((value) => ({
                  ...value,
                  kind: event.target.value as ProviderKind,
                }))
              }
            >
              {Object.entries(names).map(([value, name]) => (
                <option key={value} value={value}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Connection name</span>
            <input
              value={provider.name}
              onChange={(event) =>
                setProvider((value) => ({ ...value, name: event.target.value }))
              }
              placeholder={names[provider.kind]}
            />
          </label>
          {!subscription && (
            <>
              <label className="field">
                <span>API base URL</span>
                <input
                  required
                  type="url"
                  value={provider.baseUrl}
                  onChange={(event) =>
                    setProvider((value) => ({
                      ...value,
                      baseUrl: event.target.value,
                    }))
                  }
                  placeholder={
                    provider.kind === "litellm"
                      ? "http://localhost:4000/v1"
                      : "https://api.openai.com/v1"
                  }
                />
                <small>
                  Include the API prefix, usually /v1. Local endpoints are
                  supported.
                </small>
              </label>
              <label className="field">
                <span>API key</span>
                <input
                  type="password"
                  autoComplete="new-password"
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  placeholder={
                    existing
                      ? "Leave blank to keep the saved key"
                      : "Optional for local endpoints"
                  }
                />
                <small>
                  Credentials are managed by the native app’s secure storage.
                </small>
              </label>
            </>
          )}
          <label className="field">
            <span>
              Model IDs <span className="optional">optional</span>
            </span>
            <textarea
              rows={3}
              value={modelText}
              onChange={(event) => setModelText(event.target.value)}
              placeholder="One model ID per line, or discover models after connecting."
            />
          </label>
          {subscription && (
            <div className="notice">
              <Sparkles size={17} />
              <p>
                Save this connection, then choose Sign in to open the provider’s
                browser sign-in flow. Solar manages the native Rig session.
              </p>
            </div>
          )}
          {provider.kind === "claude" && (
            <label className="check-row warning">
              <input
                required
                type="checkbox"
                checked={accepted}
                onChange={(event) => setAccepted(event.target.checked)}
              />
              <span>
                <strong>Enable experimental Claude subscription access</strong>
                <small>
                  Anthropic does not support this third-party subscription mode.
                  Access may stop working or the account may be restricted. I
                  understand and choose to enable it. This also enables
                  experimental subscription support in Solar settings.
                </small>
              </span>
            </label>
          )}
        </div>
        <FormFooter {...status} label="Save connection" onClose={onClose} />
      </form>
    </Modal>
  );
}

export function GroupForm({
  data,
  onClose,
  onCreated,
}: {
  data: Bootstrap;
  onClose: () => void;
  onCreated: (conversation: Conversation) => void;
}) {
  const [participants, setParticipants] = useState<string[]>(
    data.orbits.filter((orbit) => orbit.enabled).map((orbit) => orbit.id),
  );
  const [lead, setLead] = useState(participants[0] ?? "");
  const [title, setTitle] = useState("");
  const [projectId, setProjectId] = useState("");
  const status = useSubmit(() => {});
  return (
    <Modal
      title="Bring your Orbits together"
      subtitle="One conversation. Different perspectives. A shared result."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void status.submit(async () => {
            if (!participants.length)
              throw new Error("Choose at least one Orbit.");
            const conversation = await api<Conversation>(
              "create_conversation",
              {
                orbitIds: participants,
                leadOrbitId: lead,
                title: title || "Team conversation",
                projectId: projectId || undefined,
              },
            );
            onCreated(conversation);
          });
        }}
      >
        <div className="form-body">
          <label className="field">
            <span>Conversation name</span>
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              placeholder="What are we working on?"
            />
          </label>
          <fieldset className="participant-options">
            <legend>Invite Orbits</legend>
            {data.orbits
              .filter((orbit) => orbit.enabled)
              .map((orbit) => (
                <label className="participant-option" key={orbit.id}>
                  <input
                    type="checkbox"
                    checked={participants.includes(orbit.id)}
                    onChange={(event) => {
                      const next = event.target.checked
                        ? [...participants, orbit.id]
                        : participants.filter((item) => item !== orbit.id);
                      setParticipants(next);
                      if (!next.includes(lead)) setLead(next[0] ?? "");
                    }}
                  />
                  <Planet color={orbit.color} name={orbit.name} size={45} />
                  <span>
                    <strong>{orbit.name}</strong>
                    <small>{orbit.role}</small>
                  </span>
                </label>
              ))}
          </fieldset>
          <label className="field">
            <span>Who should lead?</span>
            <select
              value={lead}
              required
              onChange={(event) => setLead(event.target.value)}
            >
              {data.orbits
                .filter((orbit) => participants.includes(orbit.id))
                .map((orbit) => (
                  <option value={orbit.id} key={orbit.id}>
                    {orbit.name} — {orbit.role}
                  </option>
                ))}
            </select>
            <small>
              The lead coordinates specialists and brings the result back to
              you.
            </small>
          </label>
          <label className="field">
            <span>Shared project</span>
            <select
              value={projectId}
              onChange={(event) => setProjectId(event.target.value)}
            >
              <option value="">No project</option>
              {data.projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <FormFooter {...status} label="Start conversation" onClose={onClose} />
      </form>
    </Modal>
  );
}

export function ProjectForm({
  existing,
  onClose,
  onSaved,
}: {
  existing?: Project;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [project, setProject] = useState<Project>(
    existing ?? { id: id(), name: "", directory: "", description: "" },
  );
  const status = useSubmit(onSaved);
  return (
    <Modal
      title={existing ? "Edit project" : "Make room for a project"}
      subtitle="A shared workspace for files, knowledge, and your team."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void status.submit(() => api("save_project", { project }));
        }}
      >
        <div className="form-body">
          <label className="field">
            <span>Project name</span>
            <input
              required
              value={project.name}
              onChange={(event) =>
                setProject({ ...project, name: event.target.value })
              }
              placeholder="e.g. Website refresh"
            />
          </label>
          <label className="field">
            <span>Folder on this computer</span>
            <input
              required
              value={project.directory}
              onChange={(event) =>
                setProject({ ...project, directory: event.target.value })
              }
              placeholder="/Users/you/Projects/website"
            />
            <small>Only Orbits you grant access can work in this folder.</small>
          </label>
          <label className="field">
            <span>Project context</span>
            <textarea
              rows={4}
              value={project.description}
              onChange={(event) =>
                setProject({ ...project, description: event.target.value })
              }
              placeholder="Goals, useful background, and what a good result looks like."
            />
          </label>
        </div>
        <FormFooter {...status} label="Save project" onClose={onClose} />
      </form>
    </Modal>
  );
}

export function MemoryForm({
  data,
  existing,
  onClose,
  onSaved,
}: {
  data: Bootstrap;
  existing?: Memory;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [owner, setOwner] = useState(
    existing?.orbitId
      ? `orbit:${existing.orbitId}`
      : existing?.projectId
        ? `project:${existing.projectId}`
        : data.orbits[0]
          ? `orbit:${data.orbits[0].id}`
          : "",
  );
  const [content, setContent] = useState(existing?.content ?? "");
  const status = useSubmit(onSaved);
  return (
    <Modal
      title="Something worth remembering"
      subtitle="Keep knowledge with one Orbit or share it with a project."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const [kind, ownerId] = owner.split(":");
          void status.submit(() =>
            api("save_memory", {
              memory: {
                id: existing?.id ?? id(),
                content,
                createdAt: existing?.createdAt ?? new Date().toISOString(),
                orbitId: kind === "orbit" ? ownerId : undefined,
                projectId: kind === "project" ? ownerId : undefined,
              },
            }),
          );
        }}
      >
        <div className="form-body">
          <label className="field">
            <span>Who can access this memory?</span>
            <select
              required
              value={owner}
              onChange={(event) => setOwner(event.target.value)}
            >
              <option value="" disabled>
                Choose an Orbit or project
              </option>
              <optgroup label="Private to an Orbit">
                {data.orbits.map((orbit) => (
                  <option key={orbit.id} value={`orbit:${orbit.id}`}>
                    {orbit.name}
                  </option>
                ))}
              </optgroup>
              <optgroup label="Shared project">
                {data.projects.map((project) => (
                  <option key={project.id} value={`project:${project.id}`}>
                    {project.name}
                  </option>
                ))}
              </optgroup>
            </select>
          </label>
          <label className="field">
            <span>Memory</span>
            <textarea
              required
              rows={6}
              value={content}
              onChange={(event) => setContent(event.target.value)}
              placeholder="Preferences, background, decisions, or information they should keep in mind."
            />
          </label>
        </div>
        <FormFooter {...status} label="Save memory" onClose={onClose} />
      </form>
    </Modal>
  );
}

export function RoutineForm({
  data,
  existing,
  onClose,
  onSaved,
}: {
  data: Bootstrap;
  existing?: Routine;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [routine, setRoutine] = useState<Routine>(
    existing ?? {
      id: id(),
      orbitId: data.orbits[0]?.id ?? "",
      prompt: "",
      intervalMinutes: 60,
      enabled: true,
      trigger: "interval",
    },
  );
  const status = useSubmit(onSaved);
  return (
    <Modal
      title={existing ? "Edit routine" : "A little help, on repeat"}
      subtitle="Routines run while Solar is open and your Mac is awake."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void status.submit(() => api("save_routine", { routine }));
        }}
      >
        <div className="form-body">
          <label className="field">
            <span>Orbit</span>
            <select
              required
              value={routine.orbitId}
              onChange={(event) =>
                setRoutine({ ...routine, orbitId: event.target.value })
              }
            >
              {data.orbits.map((orbit) => (
                <option key={orbit.id} value={orbit.id}>
                  {orbit.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>When should it run?</span>
            <select
              value={routine.trigger}
              onChange={(event) =>
                setRoutine({
                  ...routine,
                  trigger: event.target.value as Routine["trigger"],
                })
              }
            >
              <option value="interval">On a repeating interval</option>
              <option value="folder">When project files change</option>
              <option value="completion">After a task completes</option>
            </select>
          </label>
          {routine.trigger === "interval" && (
            <label className="field">
              <span>Repeat every (minutes)</span>
              <input
                type="number"
                min={1}
                max={43200}
                required
                value={routine.intervalMinutes}
                onChange={(event) =>
                  setRoutine({
                    ...routine,
                    intervalMinutes: Number(event.target.value),
                  })
                }
              />
            </label>
          )}
          <label className="field">
            <span>
              Project {routine.trigger === "folder" ? "" : "(optional)"}
            </span>
            <select
              required={routine.trigger === "folder"}
              value={routine.projectId ?? ""}
              onChange={(event) =>
                setRoutine({
                  ...routine,
                  projectId: event.target.value || undefined,
                })
              }
            >
              <option value="">Choose a project</option>
              {data.projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>What should they do?</span>
            <textarea
              required
              rows={4}
              value={routine.prompt}
              onChange={(event) =>
                setRoutine({ ...routine, prompt: event.target.value })
              }
              placeholder="Check the project for new test failures and summarize what changed."
            />
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={routine.enabled}
              onChange={(event) =>
                setRoutine({ ...routine, enabled: event.target.checked })
              }
            />
            <span>Enable this routine</span>
          </label>
        </div>
        <FormFooter {...status} label="Save routine" onClose={onClose} />
      </form>
    </Modal>
  );
}

export function McpForm({
  existing,
  onClose,
  onSaved,
}: {
  existing?: McpServer;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [server, setServer] = useState<McpServer>(
    existing ?? {
      id: id(),
      name: "",
      transport: "http",
      url: "",
      enabled: true,
    },
  );
  const [argsText, setArgsText] = useState(
    JSON.stringify(existing?.args ?? []),
  );
  const [secretText, setSecretText] = useState("");
  const [clearCredentials, setClearCredentials] = useState(false);
  const status = useSubmit(onSaved);
  return (
    <Modal
      title={existing ? "Edit MCP server" : "Connect more possibilities"}
      subtitle="Give your Orbits access to tools through the Model Context Protocol."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void status.submit(async () => {
            const args: unknown = JSON.parse(argsText);
            if (
              !Array.isArray(args) ||
              args.some((value) => typeof value !== "string")
            )
              throw new Error("Arguments must be a JSON array of strings.");
            let credentials: Record<string, string> | undefined;
            if (secretText.trim() && !clearCredentials) {
              const parsed: unknown = JSON.parse(secretText);
              if (
                !parsed ||
                Array.isArray(parsed) ||
                typeof parsed !== "object" ||
                Object.values(parsed).some((value) => typeof value !== "string")
              ) {
                throw new Error(
                  "Credentials must be a JSON object with string values.",
                );
              }
              credentials = parsed as Record<string, string>;
            }
            return api("save_mcp_server", {
              server: {
                ...server,
                args,
                env: server.transport === "stdio" ? credentials : undefined,
                headers: server.transport === "http" ? credentials : undefined,
                clearCredentials: clearCredentials || undefined,
              },
            });
          });
        }}
      >
        <div className="form-body">
          <label className="field">
            <span>Server name</span>
            <input
              required
              value={server.name}
              onChange={(event) =>
                setServer({ ...server, name: event.target.value })
              }
              placeholder="e.g. Documentation"
            />
          </label>
          <label className="field">
            <span>Transport</span>
            <select
              value={server.transport}
              onChange={(event) =>
                setServer({
                  ...server,
                  transport: event.target.value as McpServer["transport"],
                })
              }
            >
              <option value="http">Streamable HTTP</option>
              <option value="stdio">Local process (stdio)</option>
            </select>
          </label>
          {server.transport === "http" ? (
            <label className="field">
              <span>Server URL</span>
              <input
                required
                type="url"
                value={server.url ?? ""}
                onChange={(event) =>
                  setServer({ ...server, url: event.target.value })
                }
                placeholder="https://example.com/mcp"
              />
            </label>
          ) : (
            <>
              <label className="field">
                <span>Executable</span>
                <input
                  required
                  value={server.command ?? ""}
                  onChange={(event) =>
                    setServer({ ...server, command: event.target.value })
                  }
                  placeholder="/usr/local/bin/mcp-server"
                />
              </label>
              <label className="field">
                <span>Arguments (JSON array)</span>
                <textarea
                  rows={3}
                  value={argsText}
                  onChange={(event) => setArgsText(event.target.value)}
                  placeholder={'["--option", "value"]'}
                />
              </label>
              <div className="notice">
                <ShieldCheck size={17} />
                <p>
                  A local MCP server executes code on your computer. Connect
                  software you trust. Connected tools become available to all Orbits and follow their approval preferences.
                </p>
              </div>
            </>
          )}
          <label className="field">
            <span>
              {server.transport === "stdio"
                ? "Environment variables"
                : "HTTP headers"}{" "}
              <span className="optional">optional JSON object</span>
            </span>
            <textarea
              rows={3}
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              disabled={clearCredentials}
              value={secretText}
              onChange={(event) => setSecretText(event.target.value)}
              placeholder={
                server.transport === "stdio"
                  ? '{"API_KEY": "your-key"}'
                  : '{"Authorization": "Bearer your-token"}'
              }
            />
            <small>
              {native
                ? "Stored by Solar’s native secure credential service. Leave blank to retain saved values."
                : "The browser preview discards credentials. Secure storage is available in the native app."}
            </small>
          </label>
          {existing?.hasCredentials && (
            <>
              <div className="notice">
                <ShieldCheck size={17} />
                <p>
                  This server has saved credentials. Their values are hidden.
                </p>
              </div>
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={clearCredentials}
                  onChange={(event) =>
                    setClearCredentials(event.target.checked)
                  }
                />
                <span>Remove this server’s saved credentials</span>
              </label>
            </>
          )}
          <label className="check-row">
            <input
              type="checkbox"
              checked={server.enabled}
              onChange={(event) =>
                setServer({ ...server, enabled: event.target.checked })
              }
            />
            <span>Enable this server</span>
          </label>
        </div>
        <FormFooter {...status} label="Save server" onClose={onClose} />
      </form>
    </Modal>
  );
}
