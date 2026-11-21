import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import {
  FolderOpen,
  LoaderCircle,
  ShieldCheck,
  Shuffle,
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
const CronEditor = lazy(() => import("./CronEditor").then(module => ({ default: module.CronEditor })));
import { memoryOrbitIds, memoryProjectIds, memoryScope } from "./memoryScope";
import { Planet } from "./Planet";
import { ModelPicker } from "./ModelPicker";
import { OrbitProgress } from "./OrbitProgress";
import { orbitProfileNeedsGeneration } from "./orbitProfile";
import { generateOrbitAvatarSeed, generateOrbitName, generateOrbitPersonality } from "./orbitIdentity";
import type { ProjectLocation } from "../projectLocation";
import "./simpleOrbit.css";
import "./projectForm.css";

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
    id: id(), name: "", role: "Personal Sprite", personality: "",
    tasks: "", instructions: "", color: colors[data.orbits.length % colors.length],
    model: "", providerId: "", autonomy: "ask", toolGrants: ["*"],
    projectIds: [], enabled: true,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const request = useRef<string | null>(null);
  const mounted = useRef(true);
  const needsGeneration = orbitProfileNeedsGeneration(orbit, existing);
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
    if (request.current) return;
    cancelSave();
    onClose();
  }, [cancelSave, onClose]);
  function update<K extends keyof Orbit>(key: K, value: Orbit[K]) {
    setOrbit((current) => ({ ...current, [key]: value }));
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    if (needsGeneration && (!orbit.providerId || !orbit.model)) {
      setError("Choose a model for your Sprite.");
      return;
    }
    setBusy(true);
    setError("");
    const requestId = id();
    request.current = requestId;
    try {
      const draft = { ...orbit, name: orbit.name.trim(),
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
    <><Modal title={existing ? `${existing.name}’s profile` : "Create your Sprite"}
      subtitle={existing ? "Shape their personality, tasks, and model." : "Tell us who they are and what you’d like their help with."}
      onClose={close} wide dismissible={!busy}>
      <form onSubmit={(event) => void submit(event)} className="simple-orbit-form" inert={busy}>
        <div className="simple-orbit-heading">
          <Planet identity={orbit.avatarSeed || orbit.id} name={orbit.name || "Your Sprite"} color={orbit.color} size={64} />
          <div><strong>{orbit.name || "Your next teammate"}</strong><p>{existing ? "All tools available" : "Solar builds their instructions automatically."}</p>
            <button type="button" className="orbit-suggest-button orbit-avatar-regenerate" disabled={busy}
              onClick={() => update("avatarSeed", generateOrbitAvatarSeed(orbit.avatarSeed || orbit.id, orbit.color))}>
              <Shuffle size={13} />Regenerate avatar
            </button>
          </div>
        </div>
        <div className="form-body">
          <div className="field orbit-generated-field">
            <div className="orbit-field-heading"><label htmlFor="orbit-name">Name</label>
              <button type="button" className="orbit-suggest-button" disabled={busy} onClick={() => update("name", generateOrbitName([...data.orbits.filter((item) => item.id !== orbit.id).map((item) => item.name), orbit.name]))}><Shuffle size={13} />Suggest a name</button>
            </div>
            <input id="orbit-name" required maxLength={100} disabled={busy} autoComplete="off" placeholder="Type a name, or try a suggestion" value={orbit.name} onChange={(event) => update("name", event.target.value)} />
          </div>
          <div className="field orbit-generated-field">
            <div className="orbit-field-heading"><label htmlFor="orbit-personality">Personality</label>
              <button type="button" className="orbit-suggest-button" disabled={busy} onClick={() => update("personality", generateOrbitPersonality())}><Shuffle size={13} />Surprise me</button>
            </div>
            <textarea id="orbit-personality" required={needsGeneration} rows={3} maxLength={8000} disabled={busy}
              placeholder="Describe their personality, or try a suggestion" value={orbit.personality} onChange={(event) => update("personality", event.target.value)} />
            <small className="orbit-field-hint">Every suggestion is yours to edit.</small>
          </div>
          <label className="field"><span>Tasks</span><textarea required={needsGeneration} rows={3} maxLength={32000} disabled={busy}
            placeholder="What should they help you do? Include any important boundaries or preferences."
            value={orbit.tasks ?? ""} onChange={(event) => update("tasks", event.target.value)} /></label>
          <ModelPicker providers={data.providers} providerId={orbit.providerId} model={orbit.model}
            required={needsGeneration} disabled={busy} onChange={(selection) => setOrbit((current) => ({ ...current, ...selection }))} />
          {needsGeneration && !data.providers.some((provider) => provider.models.length) && (
            <p className="simple-orbit-note">Add a connection and discover its models in Connections before {existing ? "updating" : "creating"} a Sprite.</p>
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
          </> : <p className="simple-orbit-note">Your selected model will turn these details into working instructions. Your Sprite starts with all tools available and asks before acting. You can change its settings any time.</p>}
          {busy && !needsGeneration && <p className="simple-orbit-progress" role="status"><LoaderCircle size={16} className="spin" />Saving settings…</p>}
        </div>
        <FormFooter busy={busy} error={error} onClose={close} label={existing ? "Save Sprite" : "Create Sprite"} />
      </form>
    </Modal>{busy && needsGeneration && <OrbitProgress identity={orbit.avatarSeed || orbit.id} name={orbit.name.trim()} color={orbit.color} creating={!existing} />}</>
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
      subtitle="Choose where your Sprites think. Their work stays organized here."
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
      title="Bring your Sprites together"
      subtitle="One conversation. Different perspectives. A shared result."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void status.submit(async () => {
            if (participants.length < 2)
              throw new Error("Choose at least two Sprites for a group chat.");
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
            <legend>Invite Sprites</legend>
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
                  <Planet identity={orbit.avatarSeed || orbit.id} color={orbit.color} name={orbit.name} size={45} />
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
        <FormFooter {...status} label="Create group" onClose={onClose} />
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
    existing ? { ...existing, name: existing.directory.split(/[\\/]/).filter(Boolean).at(-1) ?? "" }
      : { id: id(), name: "", directory: "", description: "" },
  );
  const [busy, setBusy] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState("");
  const [location, setLocation] = useState<ProjectLocation | null>(null);
  const [resolving, setResolving] = useState(Boolean(existing));
  const folderRevision = useRef(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    if (!existing?.directory) return;
    let active = true;
    const revision = folderRevision.current;
    setResolving(true);
    void api<ProjectLocation>("resolve_project_directory", { directory: existing.directory })
      .then((result) => {
        if (!active || !mounted.current || revision !== folderRevision.current) return;
        setLocation(result);
        setProject((value) => ({ ...value, name: result.name, directory: result.directory }));
      })
      .catch((reason) => {
        if (active && mounted.current && revision === folderRevision.current) setError(errorText(reason));
      })
      .finally(() => {
        if (active && mounted.current && revision === folderRevision.current) setResolving(false);
      });
    return () => { active = false; };
  }, [existing?.id, existing?.directory]);
  const close = useCallback(() => {
    if (!busy && !choosing) onClose();
  }, [busy, choosing, onClose]);
  async function chooseFolder() {
    if (busy || choosing) return;
    folderRevision.current += 1;
    setResolving(false);
    setChoosing(true);
    setError("");
    try {
      const result = await api<ProjectLocation | null>("choose_project_directory", { directory: project.directory || null });
      if (result && mounted.current) {
        setLocation(result);
        setProject((value) => ({ ...value, directory: result.directory, name: result.name }));
      }
    } catch (reason) {
      if (mounted.current) setError(errorText(reason));
    } finally {
      if (mounted.current) setChoosing(false);
    }
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || choosing) return;
    folderRevision.current += 1;
    setResolving(false);
    setBusy(true);
    setError("");
    try {
      const result = await api<ProjectLocation>("resolve_project_directory", { directory: project.directory });
      if (!mounted.current) return;
      setLocation(result);
      const saved = { ...project, name: result.name, directory: result.directory };
      setProject(saved);
      await api("save_project", { project: saved });
      if (mounted.current) onSaved();
    } catch (reason) {
      if (mounted.current) setError(errorText(reason));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <Modal
      title={existing ? "Edit project" : "Add a project folder"}
      subtitle="Choose a folder and add useful context for the work."
      onClose={close}
      dismissible={!busy && !choosing}
    >
      <form className="project-form" onSubmit={(event) => void submit(event)}>
        <div className="form-body">
          <label className="field">
            <span>Project folder</span>
            <div className="project-folder-control">
            <input
              required
              disabled={busy || choosing}
              value={project.directory}
              onChange={(event) => {
                folderRevision.current += 1;
                setResolving(false);
                setLocation(null);
                setProject({ ...project, name: "", directory: event.target.value });
              }}
              placeholder={native ? "Choose a folder or paste its path" : "/Users/you/Projects/website"}
            />
            <button type="button" className="button button-secondary" disabled={busy || choosing || !native}
              onClick={() => void chooseFolder()}>
              {choosing ? <LoaderCircle size={15} className="spin" /> : <FolderOpen size={15} />}
              {choosing ? "Choosing…" : "Choose folder"}
            </button>
            </div>
            <small>If the folder is inside a Git repository, Solar uses the repository’s root folder.</small>
          </label>
          <label className="field">
            <span>Project name</span>
            <input readOnly value={project.name} placeholder="Named automatically from the project folder" />
            <small>{resolving ? "Checking project folder…" : location?.isGitRoot ? "Using the Git repository root." : "The project name matches its root folder."}</small>
          </label>
          {!native && <p className="project-folder-note">Folder selection and Git detection are available in the desktop app. Paste a path here to preview the form.</p>}
          <label className="field">
            <span>Project context</span>
            <textarea
              rows={4}
              disabled={busy || choosing}
              value={project.description}
              onChange={(event) =>
                setProject({ ...project, description: event.target.value })
              }
              placeholder="Goals, useful background, and what a good result looks like."
            />
          </label>
        </div>
        <FormFooter busy={busy || choosing} error={error} label="Save project" onClose={close} />
      </form>
    </Modal>
  );
}

export function MemoryForm({ data, existing, onClose, onSaved }: {
  data: Bootstrap; existing?: Memory; onClose: () => void; onSaved: () => void;
}) {
  const [scope, setScope] = useState<"system" | "orbit" | "project">(existing ? memoryScope(existing) : "system");
  const [orbitIds, setOrbitIds] = useState(existing ? memoryOrbitIds(existing) : []);
  const [projectIds, setProjectIds] = useState(existing ? memoryProjectIds(existing) : []);
  const [content, setContent] = useState(existing?.content ?? "");
  const status = useSubmit(onSaved);
  const choices = scope === "orbit" ? data.orbits : data.projects;
  const selected = scope === "orbit" ? orbitIds : projectIds;
  function toggle(value: string) {
    const next = selected.includes(value) ? selected.filter(item => item !== value) : [...selected, value];
    if (scope === "orbit") setOrbitIds(next); else setProjectIds(next);
  }
  return <Modal title={existing ? "Edit memory" : "Something worth remembering"}
    subtitle="Share a fact with the whole Solar System, selected Sprites, or selected projects."
    onClose={onClose} dismissible={!status.busy}>
    <form onSubmit={event => { event.preventDefault(); void status.submit(async () => {
      if (scope !== "system" && !selected.length) throw new Error(`Choose at least one ${scope === "orbit" ? "Sprite" : "project"}.`);
      return api("save_memory", { memory: { id: existing?.id ?? id(), content,
        createdAt: existing?.createdAt ?? new Date().toISOString(),
        orbitIds: scope === "orbit" ? orbitIds : [], projectIds: scope === "project" ? projectIds : [],
        orbitId: null, projectId: null } });
    }); }}>
      <div className="form-body">
        <label className="field"><span>Share with</span><select aria-label="Share with" value={scope} onChange={event => setScope(event.target.value as typeof scope)}>
          <option value="system">Entire Solar System</option><option value="orbit">Selected Sprites</option><option value="project">Selected projects</option>
        </select><small>{scope === "system" ? "Every Sprite can see and update this memory." : scope === "orbit" ? "Choose one or more Sprites that should share this memory." : "Sprites can use this memory when they have access to one of these projects."}</small></label>
        {scope !== "system" && <fieldset className="memory-share-options"><legend>{scope === "orbit" ? "Sprites" : "Projects"}</legend>
          {choices.map(choice => <label className="check-row" key={choice.id}><input type="checkbox" checked={selected.includes(choice.id)} onChange={() => toggle(choice.id)} /><span>{choice.name}</span></label>)}
          {!choices.length && <p className="subtle-label">Add {scope === "orbit" ? "a Sprite" : "a project"} first, or share with the entire Solar System.</p>}
        </fieldset>}
        <label className="field"><span>Memory</span><textarea required maxLength={16000} rows={6} value={content} onChange={event => setContent(event.target.value)} placeholder="Preferences, decisions, context, or a useful fact." /></label>
      </div><FormFooter {...status} label="Save memory" onClose={onClose} />
    </form>
  </Modal>;
}

export function RoutineForm({ existing, onClose, onSaved }: {
  data: Bootstrap; existing?: Routine; onClose: () => void; onSaved: () => void;
}) {
  const [routine, setRoutine] = useState<Routine | undefined>(existing);
  const status = useSubmit(onSaved);
  if (!routine) return <Modal title="Ask a Sprite" onClose={onClose}><div className="form-body">Describe the routine in chat and your Sprite will set it up.</div></Modal>;
  return <Modal title="Routine schedule" subtitle="Ask the Sprite in chat to change what this routine does." onClose={onClose} dismissible={!status.busy}>
    <form onSubmit={event => { event.preventDefault(); void status.submit(() => api("save_routine", { routine })); }}>
      <div className="form-body"><h3>{routine.title || routine.prompt.split("\n")[0]}</h3>
        <label className="field"><span>When should it run?</span><select value={routine.trigger} onChange={event => setRoutine({ ...routine, trigger: event.target.value as Routine["trigger"], cron: routine.cron || "0 9 * * 1-5" })}>
          {routine.trigger === "interval" && <option value="interval">Existing interval (choose a schedule to replace)</option>}
          <option value="cron">On a schedule</option><option value="folder" disabled={!routine.projectId}>When project files change</option><option value="completion">After a task completes</option>
        </select></label>
        {routine.trigger === "cron" && <Suspense fallback={<p role="status">Loading schedule editor…</p>}><CronEditor value={routine.cron || "0 9 * * 1-5"} onChange={cron => setRoutine({ ...routine, cron })} /></Suspense>}
        <label className="check-row"><input type="checkbox" checked={routine.enabled} onChange={event => setRoutine({ ...routine, enabled: event.target.checked })} /><span>Enable this routine</span></label>
      </div><FormFooter {...status} label="Save schedule" onClose={onClose} />
    </form>
  </Modal>;
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
      subtitle="Give your Sprites access to tools through the Model Context Protocol."
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
                  software you trust. Connected tools become available to all Sprites and follow their approval preferences.
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
                ? "Stored in private files under ~/.solar/credentials. Leave blank to retain saved values."
                : "The browser preview discards credentials. Private credential storage is available in the desktop app."}
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
