import { lazy, Suspense, useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import {
  Activity,
  ArrowDownToLine,
  ArrowRight,
  Bell,
  Check,
  CheckCheck,
  ChevronRight,
  CircleHelp,
  Clock3,
  Download,
  Ellipsis,
  FileJson,
  Folder,
  Globe2,
  Layers3,
  LoaderCircle,
  LockKeyhole,
  MessageCircle,
  Orbit as OrbitIcon,
  Plus,
  Power,
  Radio,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Square,
  Sun,
  Trash2,
  Unplug,
  Users,
  X,
} from "lucide-react";
import { api, native } from "./api";
import { listen } from "@tauri-apps/api/event";
import { useSolar } from "./useSolar";
import { activeTask, errorText, isProviderConfigured } from "./types";
import type {
  Conversation,
  McpServer,
  Memory,
  Orbit,
  Project,
  Provider,
  Routine,
} from "./types";
import { Planet } from "./components/Planet";
import { OrbitSettingsPanel } from "./components/OrbitSettingsPanel";
import { SolarSystem } from "./components/SolarSystem";
import { Modal } from "./components/Modal";
import { Chat } from "./components/Chat";
import { ResetSolar } from "./components/ResetSolar";
import {
  GroupForm,
  McpForm,
  MemoryForm,
  OrbitForm,
  ProjectForm,
  ProviderForm,
  RoutineForm,
} from "./components/Forms";

type Page =
  | "home"
  | "orbits"
  | "conversations"
  | "projects"
  | "connections"
  | "memory"
  | "routines"
  | "activity"
  | "settings";

const Onboarding = lazy(() => import("./onboarding/Onboarding").then(({ Onboarding }) => ({ default: Onboarding })));
type Dialog =
  | { kind: "orbit"; value?: Orbit }
  | { kind: "orbit-settings"; orbitId: string }
  | { kind: "provider"; value?: Provider }
  | { kind: "project"; value?: Project }
  | { kind: "routine"; value?: Routine }
  | { kind: "mcp"; value?: McpServer }
  | { kind: "group" | "help" }
  | { kind: "memory"; value?: Memory }
  | { kind: "export"; json: string };

const navigation: { id: Page; label: string; icon: typeof Sun }[] = [
  { id: "home", label: "Solar system", icon: Sun },
  { id: "orbits", label: "Your Orbits", icon: OrbitIcon },
  { id: "conversations", label: "Conversations", icon: MessageCircle },
  { id: "projects", label: "Projects", icon: Folder },
  { id: "memory", label: "Memory", icon: Layers3 },
  { id: "routines", label: "Routines", icon: Clock3 },
  { id: "activity", label: "Activity", icon: Activity },
];

function Empty({
  icon,
  title,
  children,
  action,
}: {
  icon: ReactNode;
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-state-icon">{icon}</div>
      <h3>{title}</h3>
      <p>{children}</p>
      {action}
    </div>
  );
}

function PageHeading({
  eyebrow,
  title,
  description,
  children,
}: {
  eyebrow?: string;
  title: string;
  description: string;
  children?: ReactNode;
}) {
  return (
    <header className="page-heading">
      <div>
        {eyebrow && <span className="eyebrow">{eyebrow}</span>}
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      <div className="page-heading-actions">{children}</div>
    </header>
  );
}

export function App() {
  const { data, loading, error: loadError, refresh } = useSolar();
  const [page, setPage] = useState<Page>("home");
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [requestedOrbitSettings, setRequestedOrbitSettings] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [toast, setToast] = useState("");
  const [search, setSearch] = useState("");
  const [previewOrbitId, setPreviewOrbitId] = useState("");
  const [replayWelcome, setReplayWelcome] = useState(false);
  const [memories, setMemories] = useState<Memory[]>([]);
  const [memoryOwner, setMemoryOwner] = useState("");
  const [logins, setLogins] = useState<
    Record<string, { pending: boolean; message?: string }>
  >({});
  const [confirm, setConfirm] = useState<{
    title: string;
    description: string;
    action: () => Promise<unknown>;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const importFile = useRef<HTMLInputElement>(null);
  const currentConversation = data.conversations.find(
    (conversation) => conversation.id === conversationId,
  );
  const running = data.tasks.filter(activeTask);
  const pendingLoginKey = Object.entries(logins)
    .filter(([, value]) => value.pending)
    .map(([providerId]) => providerId)
    .sort()
    .join(",");

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void listen<{ conversationId: string }>(
      "solar://navigate",
      ({ payload }) => {
        setPage("conversations");
        setConversationId(payload.conversationId);
        void refresh();
      },
    ).then((unlisten) => {
      if (disposed) unlisten();
      else cleanup = unlisten;
    });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [refresh]);

  useEffect(() => {
    if (!native) {
      const pendingOrbitId = sessionStorage.getItem("solar.preview.edit-orbit");
      if (pendingOrbitId) {
        sessionStorage.removeItem("solar.preview.edit-orbit");
        setRequestedOrbitSettings(pendingOrbitId);
      }
      const previewEdit = (event: Event) => {
        sessionStorage.removeItem("solar.preview.edit-orbit");
        setRequestedOrbitSettings((event as CustomEvent<{ orbitId: string }>).detail.orbitId);
        void refresh();
      };
      window.addEventListener("solar-preview-edit-orbit", previewEdit);
      return () => window.removeEventListener("solar-preview-edit-orbit", previewEdit);
    }
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void listen<{ orbitId: string }>("solar://edit-orbit", ({ payload }) => {
      setRequestedOrbitSettings(payload.orbitId);
      void refresh();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else cleanup = unlisten;
    });
    return () => { disposed = true; cleanup?.(); };
  }, [refresh]);

  useEffect(() => {
    if (!requestedOrbitSettings) return;
    if (!data.orbits.some((orbit) => orbit.id === requestedOrbitSettings)) return;
    setPage("orbits");
    setConversationId(null);
    setDialog({ kind: "orbit-settings", orbitId: requestedOrbitSettings });
    setRequestedOrbitSettings(null);
  }, [requestedOrbitSettings, data.orbits]);

  useEffect(() => {
    const pending = pendingLoginKey ? pendingLoginKey.split(",") : [];
    if (!pending.length) return;
    let disposed = false;
    const timer = window.setInterval(() => {
      for (const providerId of pending) {
        void api<{ connected: boolean; pending: boolean; message?: string }>(
          "login_status",
          { providerId },
        )
          .then((status) => {
            if (disposed) return;
            setLogins((current) => ({
              ...current,
              [providerId]: {
                pending: status.pending,
                message: status.message,
              },
            }));
            if (status.connected) {
              setToast("Subscription connected.");
              void refresh();
            } else if (!status.pending)
              setError(
                status.message || "Sign-in did not complete. Try again.",
              );
          })
          .catch((failure) => {
            if (disposed) return;
            setLogins((current) => ({
              ...current,
              [providerId]: { pending: false, message: errorText(failure) },
            }));
            setError(errorText(failure));
          });
      }
    }, 1500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [pendingLoginKey, refresh]);

  useEffect(() => {
    document.documentElement.dataset.motion = data.settings.reducedMotion
      ? "reduced"
      : "full";
  }, [data.settings.reducedMotion]);
  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(""), 4000);
    return () => clearTimeout(timer);
  }, [toast]);
  useEffect(() => {
    if (page !== "memory") return;
    let current = true;
    const [kind, ownerId] = memoryOwner.split(":");
    const scopes = memoryOwner
      ? [
          {
            orbitId: kind === "orbit" ? ownerId : undefined,
            projectId: kind === "project" ? ownerId : undefined,
          },
        ]
      : [
          ...data.orbits.map((orbit) => ({
            orbitId: orbit.id,
            projectId: undefined,
          })),
          ...data.projects.map((project) => ({
            orbitId: undefined,
            projectId: project.id,
          })),
        ];
    void Promise.all(
      scopes.map((scope) => api<Memory[]>("get_memories", scope)),
    )
      .then((results) => {
        if (current)
          setMemories([
            ...new Map(
              results.flat().map((memory) => [memory.id, memory]),
            ).values(),
          ]);
      })
      .catch((failure) => {
        if (current) setError(errorText(failure));
      });
    return () => {
      current = false;
    };
  }, [page, memoryOwner, data]);

  function go(next: Page) {
    setPage(next);
    setConversationId(null);
    setSearch("");
  }
  async function action(
    command: string,
    args: Record<string, unknown> = {},
    success?: string,
  ) {
    try {
      setError("");
      const result = await api(command, args);
      await refresh();
      if (success) setToast(success);
      return result;
    } catch (failure) {
      setError(errorText(failure));
      return undefined;
    }
  }
  async function startChat(orbit: Orbit) {
    try {
      const previous = data.conversations.find(
        (conversation) =>
          conversation.orbitIds.length === 1 &&
          conversation.orbitIds[0] === orbit.id,
      );
      const conversation =
        previous ??
        (await api<Conversation>("create_conversation", {
          orbitIds: [orbit.id],
          leadOrbitId: orbit.id,
          title: orbit.name,
          projectId: undefined,
        }));
      setPage("conversations");
      setConversationId(conversation.id);
      await refresh();
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  async function startLogin(providerId: string) {
    setError("");
    setLogins((current) => ({
      ...current,
      [providerId]: {
        pending: true,
        message: "Complete sign-in in your browser.",
      },
    }));
    try {
      await api("start_login", { providerId });
    } catch (failure) {
      setLogins((current) => ({
        ...current,
        [providerId]: { pending: false },
      }));
      setError(errorText(failure));
    }
  }
  async function exportOrbit(orbit: Orbit) {
    try {
      const result = await api<unknown>("export_orbit", { id: orbit.id });
      setDialog({
        kind: "export",
        json:
          typeof result === "string" ? result : JSON.stringify(result, null, 2),
      });
    } catch (failure) {
      setError(errorText(failure));
    }
  }
  async function importOrbit(file?: File) {
    if (!file) return;
    try {
      if (file.size > 64_000)
        throw new Error("Orbit profiles must be smaller than 64 KB.");
      await api("import_orbit", { profileJson: await file.text() });
      await refresh();
      setToast(
        "Orbit imported. Review its connection and permissions before use.",
      );
    } catch (failure) {
      setError(errorText(failure));
    }
  }
  function saved() {
    setDialog(null);
    void refresh();
  }
  const orbitName = (id: string) =>
    data.orbits.find((orbit) => orbit.id === id)?.name ?? "Orbit";
  const filteredOrbits = data.orbits.filter((orbit) =>
    `${orbit.name} ${orbit.role}`.toLowerCase().includes(search.toLowerCase()),
  );

  if (loading && !data.orbits.length) {
    return <div className="startup-black" aria-label="Starting Solar" aria-busy="true" />;
  }
  if (!loadError && (replayWelcome || !data.settings.onboardingComplete)) {
    return <Suspense fallback={<div className="startup-black" aria-label="Starting Solar" aria-busy="true" />}><Onboarding
      data={data}
      refresh={refresh}
      initialStep={replayWelcome ? "birth" : undefined}
      onNavigate={(destination) => go(destination === "orbits" ? "home" : destination)}
      onComplete={() => { void refresh().then(() => setReplayWelcome(false)); }}
    /></Suspense>;
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <button
          className="brand"
          onClick={() => go("home")}
          aria-label="Solar home"
        >
          <Sun size={29} strokeWidth={1.6} />
          <span>
            solar<span className="brand-dot">.</span>
          </span>
        </button>
        <div className="workspace-chip">
          <span className="workspace-avatar">Y</span>
          <span>
            Your workspace<small>Local · Personal</small>
          </span>
          <LockKeyhole size={13} />
        </div>
        <span className="nav-label">WORKSPACE</span>
        <nav aria-label="Workspace navigation">
          {navigation.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              aria-label={label}
              className={`nav-item ${page === id ? "selected" : ""}`}
              onClick={() => go(id)}
            >
              <Icon size={18} />
              <span>{label}</span>
              {id === "orbits" && (
                <span className="nav-count">{data.orbits.length}</span>
              )}
              {id === "activity" && running.length > 0 && (
                <span className="activity-count">{running.length}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-team">
          <div className="sidebar-section-heading">
            <span className="nav-label">YOUR TEAM</span>
            <button
              className="icon-button"
              onClick={() => setDialog({ kind: "orbit" })}
              title="Create an Orbit"
              aria-label="Create an Orbit"
            >
              <Plus size={15} />
            </button>
          </div>
          {data.orbits.slice(0, 5).map((orbit) => (
            <button
              className="sidebar-orbit"
              aria-label={`Chat with ${orbit.name}`}
              key={orbit.id}
              onClick={() => void startChat(orbit)}
            >
              <Planet color={orbit.color} name={orbit.name} size={32} />
              <span>{orbit.name}</span>
              <span
                className={`tiny-dot ${running.some((task) => task.orbitId === orbit.id) ? "working" : ""}`}
              />
            </button>
          ))}
          {!data.orbits.length && (
            <button
              className="sidebar-orbit"
              onClick={() => setDialog({ kind: "orbit" })}
            >
              <Plus size={18} />
              <span>Create your first Orbit</span>
            </button>
          )}
        </div>
        <div className="sidebar-bottom">
          <button
            className={`nav-item ${page === "connections" ? "selected" : ""}`}
            aria-label="Connections"
            onClick={() => go("connections")}
          >
            <Unplug size={18} />
            <span>Connections</span>
          </button>
          <button
            className={`nav-item ${page === "settings" ? "selected" : ""}`}
            aria-label="Settings"
            onClick={() => go("settings")}
          >
            <Settings2 size={18} />
            <span>Settings</span>
          </button>
          <div className="sidebar-footnote">
            <span className="online-indicator" />
            <span>Running on your computer</span>
            <button
              className="icon-button"
              aria-label="About Solar"
              onClick={() => setDialog({ kind: "help" })}
            >
              <CircleHelp size={14} />
            </button>
          </div>
        </div>
      </aside>
      <main className="main-area">
        <div className="topbar">
          <div className="breadcrumb">
            Workspace
            <ChevronRight size={12} />
            <span>
              {navigation.find((item) => item.id === page)?.label ??
                (page === "connections" ? "Connections" : "Settings")}
            </span>
          </div>
          <div className="topbar-actions">
            {!native && <span className="preview-badge">Browser preview</span>}
            <span className="local-badge">
              <LockKeyhole size={12} />
              Local workspace
            </span>
            <button
              className="notification-button"
              aria-label="View pending approvals"
              onClick={() => go("activity")}
            >
              <Bell size={17} />
              {data.approvals.length > 0 && (
                <span>{data.approvals.length}</span>
              )}
            </button>
          </div>
        </div>
        {(error || loadError) && (
          <div className="global-error" role="alert">
            <span>{error || loadError}</span>
            <button
              onClick={() => {
                setError("");
                void refresh();
              }}
            >
              Dismiss
            </button>
          </div>
        )}
        {loading ? (
          <div className="loading-state">
            <Planet color="#e8aa62" sun size={82} />
            <LoaderCircle size={20} className="spin" />
            <span>Opening your Solar system…</span>
          </div>
        ) : (
          <>
            {page === "home" && (
              <SolarSystem
                orbits={data.orbits}
                onSelectOrbit={(orbit) => void startChat(orbit)}
                onCreateOrbit={() => setDialog({ kind: "orbit" })}
                reducedMotion={data.settings.reducedMotion}
              />
            )}
            {page === "orbits" && (
              <div className="page-content">
                <PageHeading
                  eyebrow="YOUR PERSONAL TEAM"
                  title="Your Orbits"
                  description={data.orbits.length ? "Choose an Orbit to chat, or bring a team together." : "Create your first Orbit and make it your own."}
                >
                  <button
                    className="button button-quiet"
                    onClick={() => setDialog({ kind: "group" })}
                    disabled={!data.orbits.length}
                  >
                    <Users size={16} />
                    Group chat
                  </button>
                  <button
                    className="button button-primary"
                    onClick={() => setDialog({ kind: "orbit" })}
                  >
                    <Plus size={16} />
                    Create Orbit
                  </button>
                </PageHeading>
                <section className="universe-banner">
                  <div className="universe-copy">
                    <span className="banner-kicker">
                      <span className="online-indicator" />A HOME FOR YOUR
                      ORBITS
                    </span>
                    <h2>
                      Your Orbits,
                      <br />
                      on your desktop.
                    </h2>
                    <p>
                      Give each Orbit a specialty, a personality, and a place on
                      your desktop.
                    </p>
                    <button
                      className="banner-link"
                      onClick={() =>
                        void action(
                          "set_desktop_expanded",
                          { expanded: true },
                          native
                            ? "Your Orbits are on the desktop."
                            : "Desktop characters are available in the native app.",
                        )
                      }
                    >
                      Bring Orbits to desktop <ArrowRight size={15} />
                    </button>
                  </div>
                  <div className="universe-art" aria-hidden="true">
                    <div className="orbital-path path-one" />
                    <div className="orbital-path path-two" />
                    <div className="orbital-path path-three" />
                    <div className="banner-sun">
                      <Planet sun color="#e8aa62" size={118} />
                    </div>
                    {data.orbits.slice(0, 3).map((orbit, index) => (
                      <div
                        className={`banner-planet banner-planet-${index}`}
                        key={orbit.id}
                      >
                        <Planet
                          color={orbit.color}
                          size={index === 1 ? 69 : 55}
                        />
                      </div>
                    ))}
                    <span className="star star-one">✦</span>
                    <span className="star star-two">✧</span>
                    <span className="star star-three">·</span>
                  </div>
                </section>
                <div className="workspace-stats">
                  <div>
                    <span className="stat-icon">
                      <OrbitIcon size={17} />
                    </span>
                    <strong>{data.orbits.length}</strong>
                    <span>Orbits in your team</span>
                  </div>
                  <div>
                    <span className="stat-icon green">
                      <Activity size={17} />
                    </span>
                    <strong>{running.length}</strong>
                    <span>working right now</span>
                  </div>
                  <div>
                    <span className="stat-icon purple">
                      <Unplug size={17} />
                    </span>
                    <strong>
                      {data.providers.filter(isProviderConfigured).length}
                    </strong>
                    <span>model connections</span>
                  </div>
                </div>
                <div className="section-heading">
                  <h2>
                    Meet your team <span>{data.orbits.length}</span>
                  </h2>
                  <div className="section-actions">
                    <div className="search-input">
                      <Search size={15} />
                      <input
                        aria-label="Find an Orbit"
                        placeholder="Find an Orbit…"
                        value={search}
                        onChange={(event) => setSearch(event.target.value)}
                      />
                    </div>
                    <button
                      className="icon-button"
                      aria-label="Import an Orbit profile"
                      title="Import Orbit"
                      onClick={() => importFile.current?.click()}
                    >
                      <ArrowDownToLine size={17} />
                    </button>
                  </div>
                </div>
                <div className="orbit-grid">
                  {filteredOrbits.map((orbit) => {
                    const isWorking = running.some(
                      (task) => task.orbitId === orbit.id,
                    );
                    const isConnected = data.providers.some(
                      (provider) =>
                        provider.id === orbit.providerId &&
                        isProviderConfigured(provider),
                    );
                    return (
                      <article
                        className="orbit-card"
                        key={orbit.id}
                        style={
                          { "--orbit-color": orbit.color } as CSSProperties
                        }
                      >
                        <div className="orbit-card-top">
                          <span
                            className={`status-pill ${isWorking ? "working" : ""}`}
                          >
                            <span />
                            {!orbit.enabled
                              ? "Paused"
                              : isWorking
                                ? "Working"
                                : isConnected
                                  ? "Ready to help"
                                  : "Needs a model"}
                          </span>
                          <details className="card-menu">
                            <summary
                              className="icon-button"
                              aria-label={`Actions for ${orbit.name}`}
                            >
                              <Ellipsis size={18} />
                            </summary>
                            <div>
                              <button
                                onClick={() =>
                                  setDialog({ kind: "orbit", value: orbit })
                                }
                              >
                                <Settings2 size={14} />
                                Edit Orbit
                              </button>
                              <button onClick={() => void exportOrbit(orbit)}>
                                <FileJson size={14} />
                                Export profile
                              </button>
                              <button
                                onClick={() =>
                                  void action("save_orbit", {
                                    orbit: {
                                      ...orbit,
                                      enabled: !orbit.enabled,
                                    },
                                  })
                                }
                              >
                                <Power size={14} />
                                {orbit.enabled ? "Pause" : "Enable"}
                              </button>
                              <button
                                className="danger-text"
                                onClick={() =>
                                  setConfirm({
                                    title: `Remove ${orbit.name}?`,
                                    description:
                                      "Remove this Orbit from your team. Export its profile first if you want a copy.",
                                    action: () =>
                                      api("delete_orbit", { id: orbit.id }),
                                  })
                                }
                              >
                                <Trash2 size={14} />
                                Remove
                              </button>
                            </div>
                          </details>
                        </div>
                        <div className="card-planet">
                          <Planet
                            color={orbit.color}
                            name={orbit.name}
                            size={103}
                            busy={isWorking}
                          />
                        </div>
                        <h3>{orbit.name}</h3>
                        <span className="orbit-role">{orbit.role}</span>
                        <p className="orbit-personality">
                          {orbit.personality || orbit.tasks || orbit.role}
                        </p>
                        <div className="tool-tags">
                          <span>All tools</span>
                          <span>{orbit.autonomy === "ask" ? "Ask first" : orbit.autonomy === "trusted" ? "Trusted" : "Workspace"}</span>
                        </div>
                        <div className="orbit-card-footer">
                          <button onClick={() => void startChat(orbit)}>
                            <MessageCircle size={15} />
                            Chat with {orbit.name}
                            <ArrowRight size={14} />
                          </button>
                        </div>
                      </article>
                    );
                  })}
                  <button
                    className="add-orbit-card"
                    onClick={() => setDialog({ kind: "orbit" })}
                  >
                    <span>
                      <Plus size={23} />
                    </span>
                    <strong>Meet your next Orbit</strong>
                    <p>
                      A new specialty.
                      <br />A new perspective.
                    </p>
                    <span className="add-orbit-link">
                      Create an Orbit <ArrowRight size={13} />
                    </span>
                  </button>
                </div>
                {search && filteredOrbits.length === 0 && (
                  <p className="muted centered">No Orbits match “{search}”.</p>
                )}
                <div className="bottom-note">
                  <ShieldCheck size={15} />
                  <span>
                    Your Orbits have their own memories. Shared projects bring
                    them together.
                  </span>
                </div>
              </div>
            )}
            {page === "conversations" &&
              (currentConversation ? (
                <Chat
                  conversation={currentConversation}
                  data={data}
                  onBack={() => setConversationId(null)}
                  onConnections={() => go("connections")}
                  onOrbitChanged={() => { setConversationId(null); void refresh(); }}
                />
              ) : (
                <div className="page-content">
                  <PageHeading
                    title="Conversations"
                    description="Pick up where you left off, with one Orbit or the whole team."
                  >
                    <button
                      className="button button-primary"
                      disabled={!data.orbits.length}
                      onClick={() => setDialog({ kind: "group" })}
                    >
                      <Plus size={16} />
                      New group chat
                    </button>
                  </PageHeading>
                  <SolarSystem
                    orbits={data.orbits}
                    onSelectOrbit={(orbit) => void startChat(orbit)}
                    onCreateOrbit={() => setDialog({ kind: "orbit" })}
                    reducedMotion={data.settings.reducedMotion}
                    compact
                  />
                  {data.conversations.length ? (
                    <div className="conversation-list">
                      {data.conversations.map((conversation) => (
                        <button
                          key={conversation.id}
                          className="conversation-row"
                          onClick={() => setConversationId(conversation.id)}
                        >
                          <div className="conversation-planets">
                            {conversation.orbitIds
                              .slice(0, 3)
                              .map((orbitId) => (
                                <Planet
                                  key={orbitId}
                                  color={
                                    data.orbits.find(
                                      (orbit) => orbit.id === orbitId,
                                    )?.color
                                  }
                                  size={43}
                                />
                              ))}
                          </div>
                          <div>
                            <h3>{conversation.title}</h3>
                            <p>
                              {conversation.orbitIds.map(orbitName).join(", ")}
                            </p>
                          </div>
                          <time>
                            {new Date(
                              conversation.createdAt,
                            ).toLocaleDateString([], {
                              month: "short",
                              day: "numeric",
                            })}
                          </time>
                          <ChevronRight size={18} />
                        </button>
                      ))}
                    </div>
                  ) : null}
                </div>
              ))}
            {page === "projects" && (
              <div className="page-content">
                <PageHeading
                  title="Projects"
                  description="Your project folders and the context that helps work move forward."
                >
                  <button
                    className="button button-primary"
                    onClick={() => setDialog({ kind: "project" })}
                  >
                    <Plus size={16} />
                    Add folder
                  </button>
                </PageHeading>
                {data.projects.length ? (
                  <div className="project-grid">
                    {data.projects.map((project) => (
                      <article
                        className="surface-card project-card"
                        key={project.id}
                      >
                        <div className="surface-card-title">
                          <span className="surface-icon">
                            <Folder size={21} />
                          </span>
                          <button
                            className="icon-button"
                            onClick={() =>
                              setDialog({ kind: "project", value: project })
                            }
                            aria-label={`Edit ${project.name}`}
                          >
                            <Settings2 size={16} />
                          </button>
                        </div>
                        <h3>{project.name}</h3>
                        <p>
                          {project.description || "Project folder"}
                        </p>
                        <code>{project.directory}</code>
                      </article>
                    ))}
                  </div>
                ) : (
                  <Empty
                    icon={<Folder size={29} />}
                    title="Keep your project folders together"
                    action={
                      <button
                        className="button button-primary"
                        onClick={() => setDialog({ kind: "project" })}
                      >
                        <Plus size={15} />
                        Add your first folder
                      </button>
                    }
                  >
                    Choose a local folder and add goals, background, or useful notes.
                    Solar names the project after its root folder.
                  </Empty>
                )}
              </div>
            )}
            {page === "connections" && (
              <div className="page-content">
                <PageHeading
                  title="Connections"
                  description="Models for thinking. Tools for getting things done."
                >
                  <button
                    className="button button-primary"
                    onClick={() => setDialog({ kind: "provider" })}
                  >
                    <Plus size={16} />
                    Add model connection
                  </button>
                </PageHeading>
                <div className="section-heading">
                  <h2>Model connections</h2>
                  <span className="subtle-label">
                    Credentials stay in secure local storage
                  </span>
                </div>
                {data.providers.length ? (
                  <div className="connection-grid">
                    {data.providers.map((provider) => (
                      <article
                        className="surface-card connection-card"
                        key={provider.id}
                      >
                        <div className="surface-card-title">
                          <span className="provider-symbol">
                            {provider.kind === "claude" ? (
                              "✳"
                            ) : provider.kind === "chatgpt" ? (
                              "◎"
                            ) : (
                              <Globe2 size={22} />
                            )}
                          </span>
                          <span
                            className={`status-pill ${provider.connected ? "connected" : ""}`}
                          >
                            <span />
                            {provider.connected
                              ? "Connected"
                              : isProviderConfigured(provider)
                                ? "Configured"
                                : "Not connected"}
                          </span>
                        </div>
                        <h3>{provider.name}</h3>
                        <p>
                          {provider.kind === "chatgpt"
                            ? "ChatGPT subscription · native Rig"
                            : provider.kind === "claude"
                              ? "Claude subscription · experimental"
                              : provider.baseUrl}
                        </p>
                        {provider.experimental && (
                          <span className="experimental-pill">
                            Experimental · unsupported provider mode
                          </span>
                        )}
                        <div className="model-chips">
                          {provider.models.slice(0, 3).map((model) => (
                            <span key={model}>{model}</span>
                          ))}
                          {provider.models.length > 3 && (
                            <span>+{provider.models.length - 3} models</span>
                          )}
                        </div>
                        <div className="button-row wrap">
                          {["chatgpt", "claude"].includes(provider.kind) &&
                            !provider.connected && (
                              <button
                                className="button button-primary button-small"
                                disabled={logins[provider.id]?.pending}
                                onClick={() => void startLogin(provider.id)}
                              >
                                {logins[provider.id]?.pending ? (
                                  <>
                                    <LoaderCircle className="spin" size={13} />
                                    Signing in…
                                  </>
                                ) : (
                                  <>
                                    Sign in <ArrowRight size={13} />
                                  </>
                                )}
                              </button>
                            )}
                          <button
                            className="button button-secondary button-small"
                            onClick={() =>
                              void action(
                                "discover_models",
                                { id: provider.id },
                                "Model list refreshed.",
                              )
                            }
                          >
                            Discover models
                          </button>
                          <button
                            className="icon-button"
                            aria-label={`Edit ${provider.name}`}
                            onClick={() =>
                              setDialog({ kind: "provider", value: provider })
                            }
                          >
                            <Settings2 size={15} />
                          </button>
                          {provider.connected && (
                            <button
                              className="icon-button"
                              aria-label={`Disconnect ${provider.name}`}
                              onClick={() =>
                                setConfirm({
                                  title: `Disconnect ${provider.name}?`,
                                  description:
                                    "Remove this connection’s stored credentials. Its configuration will remain.",
                                  action: () =>
                                    api("logout_provider", { id: provider.id }),
                                })
                              }
                            >
                              <Unplug size={15} />
                            </button>
                          )}
                        </div>
                      </article>
                    ))}
                  </div>
                ) : (
                  <Empty
                    icon={<Sparkles size={29} />}
                    title="Choose where your Orbits think"
                    action={
                      <button
                        className="button button-primary"
                        onClick={() => setDialog({ kind: "provider" })}
                      >
                        <Plus size={15} />
                        Connect your first model
                      </button>
                    }
                  >
                    Connect LiteLLM, an OpenAI-compatible endpoint, or a
                    supported subscription. Local model endpoints work too.
                  </Empty>
                )}
                <div className="section-heading section-spaced">
                  <h2>MCP tool servers</h2>
                  <button
                    className="button button-secondary button-small"
                    onClick={() => setDialog({ kind: "mcp" })}
                  >
                    <Plus size={15} />
                    Add MCP server
                  </button>
                </div>
                {(data.mcpServers ?? []).length ? (
                  <div className="connection-grid">
                    {data.mcpServers?.map((server) => (
                      <article className="surface-card" key={server.id}>
                        <div className="surface-card-title">
                          <Radio size={21} />
                          <span
                            className={`status-pill ${server.connected ? "connected" : ""}`}
                          >
                            <span />
                            {server.connected
                              ? "Connected"
                              : server.enabled
                                ? "Enabled"
                                : "Disabled"}
                          </span>
                        </div>
                        <h3>{server.name}</h3>
                        <p>
                          {server.transport === "http"
                            ? server.url
                            : `${server.command} ${(server.args ?? []).join(" ")}`}
                        </p>
                        <div className="button-row">
                          <button
                            className="button button-secondary button-small"
                            onClick={() =>
                              void action(
                                "connect_mcp",
                                { id: server.id },
                                "MCP connection checked.",
                              )
                            }
                          >
                            Connect
                          </button>
                          <button
                            className="icon-button"
                            aria-label={`Edit ${server.name}`}
                            onClick={() =>
                              setDialog({ kind: "mcp", value: server })
                            }
                          >
                            <Settings2 size={15} />
                          </button>
                          <button
                            className="icon-button danger-text"
                            aria-label={`Remove ${server.name}`}
                            onClick={() =>
                              setConfirm({
                                title: `Remove ${server.name}?`,
                                description:
                                  "Disconnect this MCP tool server and remove its configuration.",
                                action: () =>
                                  api("delete_mcp_server", { id: server.id }),
                              })
                            }
                          >
                            <Trash2 size={15} />
                          </button>
                        </div>
                      </article>
                    ))}
                  </div>
                ) : (
                  <div className="inline-empty">
                    <Radio size={19} />
                    <p>
                      Connect standard MCP servers to extend your Orbits’ tools.
                    </p>
                  </div>
                )}
              </div>
            )}
            {page === "memory" && (
              <div className="page-content">
                <PageHeading
                  title="Memory"
                  description="Review what your Orbits remember and what projects share."
                >
                  <button
                    className="button button-primary"
                    disabled={!data.orbits.length && !data.projects.length}
                    onClick={() => setDialog({ kind: "memory" })}
                  >
                    <Plus size={16} />
                    Add memory
                  </button>
                </PageHeading>
                <div className="filter-bar">
                  <div className="search-input">
                    <Search size={15} />
                    <input
                      placeholder="Search memories…"
                      aria-label="Search memories"
                      value={search}
                      onChange={(event) => setSearch(event.target.value)}
                    />
                  </div>
                  <select
                    value={memoryOwner}
                    onChange={(event) => setMemoryOwner(event.target.value)}
                    aria-label="Filter memories by owner"
                  >
                    <option value="">All memories</option>
                    <optgroup label="Private to an Orbit">
                      {data.orbits.map((orbit) => (
                        <option key={orbit.id} value={`orbit:${orbit.id}`}>
                          {orbit.name}
                        </option>
                      ))}
                    </optgroup>
                    <optgroup label="Shared projects">
                      {data.projects.map((project) => (
                        <option
                          key={project.id}
                          value={`project:${project.id}`}
                        >
                          {project.name}
                        </option>
                      ))}
                    </optgroup>
                  </select>
                </div>
                {memories.length ? (
                  <div className="memory-grid">
                    {memories
                      .filter((memory) =>
                        memory.content
                          .toLowerCase()
                          .includes(search.toLowerCase()),
                      )
                      .map((memory) => (
                        <article
                          className="surface-card memory-card"
                          key={memory.id}
                        >
                          <div className="surface-card-title">
                            <span className="memory-owner">
                              {memory.projectId ? (
                                <Users size={13} />
                              ) : (
                                <LockKeyhole size={13} />
                              )}
                              {memory.projectId
                                ? (data.projects.find(
                                    (project) =>
                                      project.id === memory.projectId,
                                  )?.name ?? "Shared project")
                                : orbitName(memory.orbitId ?? "")}
                            </span>
                            <button
                              className="icon-button"
                              aria-label="Edit memory"
                              onClick={() =>
                                setDialog({ kind: "memory", value: memory })
                              }
                            >
                              <Settings2 size={14} />
                            </button>
                            <button
                              className="icon-button"
                              aria-label="Delete memory"
                              onClick={() =>
                                setConfirm({
                                  title: "Forget this memory?",
                                  description:
                                    "This removes the saved memory. Existing conversation messages remain.",
                                  action: () =>
                                    api("delete_memory", { id: memory.id }),
                                })
                              }
                            >
                              <Trash2 size={14} />
                            </button>
                          </div>
                          <p>{memory.content}</p>
                          <time>
                            {new Date(memory.createdAt).toLocaleDateString()}
                          </time>
                        </article>
                      ))}
                  </div>
                ) : (
                  <Empty
                    icon={<Layers3 size={29} />}
                    title="A little context goes a long way"
                  >
                    Save preferences, decisions, and useful facts. Private
                    memories belong to one Orbit. Project memories are shared
                    with its team.
                  </Empty>
                )}
              </div>
            )}
            {page === "routines" && (
              <div className="page-content">
                <PageHeading
                  title="Routines"
                  description="Choose the work your Orbits can start on their own."
                >
                  <button
                    className="button button-primary"
                    disabled={!data.orbits.length}
                    onClick={() => setDialog({ kind: "routine" })}
                  >
                    <Plus size={16} />
                    New routine
                  </button>
                </PageHeading>
                <div className="notice page-notice">
                  <Clock3 size={18} />
                  <p>
                    Routines run while Solar is open and your Mac is awake,
                    within the Orbit’s existing permissions.
                  </p>
                </div>
                {data.routines.length ? (
                  <div className="routine-list">
                    {data.routines.map((routine) => (
                      <article
                        className="surface-card routine-card"
                        key={routine.id}
                      >
                        <Planet
                          color={
                            data.orbits.find(
                              (orbit) => orbit.id === routine.orbitId,
                            )?.color
                          }
                          size={58}
                        />
                        <div>
                          <span className="eyebrow">
                            {orbitName(routine.orbitId)} ·{" "}
                            {routine.trigger === "interval"
                              ? `Every ${routine.intervalMinutes} minutes`
                              : routine.trigger === "folder"
                                ? "When project files change"
                                : "After task completion"}
                          </span>
                          <h3>{routine.prompt}</h3>
                          <span className="subtle-label">
                            {routine.projectId
                              ? data.projects.find(
                                  (project) => project.id === routine.projectId,
                                )?.name
                              : "Private Orbit routine"}
                          </span>
                        </div>
                        <button
                          className={`switch ${routine.enabled ? "on" : ""}`}
                          role="switch"
                          aria-checked={routine.enabled}
                          aria-label={`Enable routine ${routine.prompt}`}
                          onClick={() =>
                            void action("save_routine", {
                              routine: {
                                ...routine,
                                enabled: !routine.enabled,
                              },
                            })
                          }
                        >
                          <span />
                        </button>
                        <button
                          className="icon-button"
                          aria-label="Edit routine"
                          onClick={() =>
                            setDialog({ kind: "routine", value: routine })
                          }
                        >
                          <Settings2 size={16} />
                        </button>
                      </article>
                    ))}
                  </div>
                ) : (
                  <Empty
                    icon={<Clock3 size={29} />}
                    title="Helpful habits, on your terms"
                    action={
                      <button
                        className="button button-primary"
                        disabled={!data.orbits.length}
                        onClick={() => setDialog({ kind: "routine" })}
                      >
                        Create a routine
                      </button>
                    }
                  >
                    Schedule a check-in, watch a project folder, or follow up
                    after a task. You decide when work begins.
                  </Empty>
                )}
              </div>
            )}
            {page === "activity" && (
              <div className="page-content">
                <PageHeading
                  title="Activity"
                  description="See who’s working, what’s waiting, and how things turned out."
                >
                  <span className="status-pill working">
                    <span />
                    {running.length} active
                  </span>
                </PageHeading>
                {data.approvals.map((approval) => (
                  <article key={approval.id} className="approval-card">
                    <div className="approval-title">
                      <ShieldCheck size={18} />
                      <strong>
                        {orbitName(approval.orbitId)} requests {approval.tool}
                      </strong>
                      <span className="experimental-pill">
                        Your approval needed
                      </span>
                    </div>
                    <pre>{JSON.stringify(approval.arguments, null, 2)}</pre>
                    <div className="button-row">
                      <button
                        className="button button-secondary"
                        onClick={() =>
                          void action("resolve_approval", {
                            id: approval.id,
                            allow: false,
                          })
                        }
                      >
                        Decline
                      </button>
                      <button
                        className="button button-primary"
                        onClick={() =>
                          void action("resolve_approval", {
                            id: approval.id,
                            allow: true,
                          })
                        }
                      >
                        <Check size={15} />
                        Allow once
                      </button>
                    </div>
                  </article>
                ))}
                {data.tasks.length ? (
                  <div className="activity-list">
                    {[...data.tasks].reverse().map((task) => (
                      <article className="activity-row" key={task.id}>
                        <span className={`activity-state ${task.status}`}>
                          {activeTask(task) ? (
                            <LoaderCircle size={17} className="spin" />
                          ) : task.status === "completed" ? (
                            <CheckCheck size={18} />
                          ) : (
                            <Activity size={17} />
                          )}
                        </span>
                        <div>
                          <h3>{task.summary || "Orbit task"}</h3>
                          <p>
                            {orbitName(task.orbitId)} <span>·</span>{" "}
                            {task.status.replaceAll("_", " ")}
                            {task.error && (
                              <span className="danger-text">
                                {" "}
                                · {task.error}
                              </span>
                            )}
                          </p>
                        </div>
                        <time>
                          {new Date(task.createdAt).toLocaleTimeString([], {
                            hour: "2-digit",
                            minute: "2-digit",
                          })}
                        </time>
                        {activeTask(task) && (
                          <button
                            className="icon-button"
                            onClick={() =>
                              void action("stop_task", { taskId: task.id })
                            }
                            aria-label="Stop task"
                          >
                            <Square size={14} />
                          </button>
                        )}
                      </article>
                    ))}
                  </div>
                ) : (
                  <Empty
                    icon={<Activity size={29} />}
                    title="Your team’s work appears here"
                  >
                    Tool requests, running tasks, and completed work will be
                    visible as your Orbits get started.
                  </Empty>
                )}
              </div>
            )}
            {page === "settings" && (
              <div className="page-content settings-page">
                <PageHeading
                  title="Settings"
                  description="Make Solar comfortable on your desktop."
                />
                <section className="settings-section">
                  <h2>Desktop companions</h2>
                  <SettingsRow
                    title="Show desktop companions"
                    description="Keep the sun within reach while Solar runs."
                  >
                    <button
                      className={`switch ${data.settings.desktopEnabled !== false ? "on" : ""}`}
                      role="switch"
                      aria-checked={data.settings.desktopEnabled !== false}
                      aria-label="Show desktop companions"
                      onClick={() =>
                        void action("save_settings", {
                          settings: {
                            ...data.settings,
                            desktopEnabled:
                              data.settings.desktopEnabled === false,
                          },
                        })
                      }
                    >
                      <span />
                    </button>
                  </SettingsRow>
                  <SettingsRow
                    title="Orbit chimes"
                    description="A short, distinct melody when an Orbit needs you while companions are collapsed."
                  >
                    <button
                      className={`switch ${data.settings.attentionSound !== false ? "on" : ""}`}
                      role="switch"
                      aria-checked={data.settings.attentionSound !== false}
                      aria-label="Orbit chimes"
                      onClick={() =>
                        void action("save_settings", {
                          settings: {
                            ...data.settings,
                            attentionSound: data.settings.attentionSound === false,
                          },
                        })
                      }
                    >
                      <span />
                    </button>
                  </SettingsRow>
                  <SettingsRow
                    title="Chime volume"
                    description="A gentle nudge. Your Mac’s system volume still applies."
                  >
                    <label className="attention-volume">
                      <input
                        type="range"
                        min="0"
                        max="1"
                        step="0.05"
                        aria-label="Chime volume"
                        value={Number(data.settings.attentionVolume ?? 0.35)}
                        onChange={(event) =>
                          void action("save_settings", {
                            settings: {
                              ...data.settings,
                              attentionVolume: Number(event.target.value),
                            },
                          })
                        }
                      />
                      <output>{Math.round(Number(data.settings.attentionVolume ?? 0.35) * 100)}%</output>
                    </label>
                  </SettingsRow>
                  <SettingsRow
                    title="Preview an Orbit’s nudge"
                    description={native ? "See the top-of-screen peek and hear its generated melody." : "Preview the top-of-screen peek. Chimes play in the desktop app."}
                  >
                    <div className="attention-preview-control">
                      <select
                        aria-label="Orbit to preview"
                        value={data.orbits.some((orbit) => orbit.id === previewOrbitId && orbit.enabled) ? previewOrbitId : data.orbits.find((orbit) => orbit.enabled)?.id ?? ""}
                        onChange={(event) => setPreviewOrbitId(event.target.value)}
                        disabled={!data.orbits.some((orbit) => orbit.enabled)}
                      >
                        {data.orbits.filter((orbit) => orbit.enabled).map((orbit) => (
                          <option key={orbit.id} value={orbit.id}>{orbit.name}</option>
                        ))}
                      </select>
                      <button
                        className="button button-secondary"
                        disabled={!data.orbits.some((orbit) => orbit.enabled)}
                        onClick={() => {
                          const orbit = data.orbits.find((item) => item.id === previewOrbitId && item.enabled) ?? data.orbits.find((item) => item.enabled);
                          if (orbit) void action("preview_attention", { orbitId: orbit.id });
                        }}
                      >
                        <Bell size={14} /> Preview
                      </button>
                    </div>
                  </SettingsRow>
                  <SettingsRow
                    title="Reduce motion"
                    description="Use gentler transitions and pause decorative animation."
                  >
                    <button
                      className={`switch ${data.settings.reducedMotion ? "on" : ""}`}
                      role="switch"
                      aria-checked={Boolean(data.settings.reducedMotion)}
                      aria-label="Reduce motion"
                      onClick={() =>
                        void action("save_settings", {
                          settings: {
                            ...data.settings,
                            reducedMotion: !data.settings.reducedMotion,
                          },
                        })
                      }
                    >
                      <span />
                    </button>
                  </SettingsRow>
                  <SettingsRow
                    title="Companion visibility"
                    description="Above ordinary applications, hidden during fullscreen use."
                  >
                    <span className="value-label">Across Spaces</span>
                  </SettingsRow>
                </section>
                <section className="settings-section">
                  <h2>Welcome experience</h2>
                  <SettingsRow title="Sound effects" description="Clicks, Solar’s introduction, and Orbit sound feedback.">
                    <button className={`switch ${data.settings.soundEffects !== false ? "on" : ""}`} role="switch" aria-checked={data.settings.soundEffects !== false} aria-label="Sound effects" onClick={() => void action("save_settings", { settings: { ...data.settings, soundEffects: data.settings.soundEffects === false } })}><span /></button>
                  </SettingsRow>
                  <SettingsRow title="Sound effects volume" description="Intro and click volume. Orbit chimes have their own volume.">
                    <label className="attention-volume">
                      <input type="range" min="0" max="0.5" step="0.01" aria-label="Sound effects volume" value={Number(data.settings.soundEffectsVolume ?? 0.25)} onChange={(event) => void action("save_settings", { settings: { ...data.settings, soundEffectsVolume: Number(event.target.value) } })} />
                      <output>{Math.round(Number(data.settings.soundEffectsVolume ?? 0.25) * 200)}%</output>
                    </label>
                  </SettingsRow>
                  <SettingsRow title="Welcome tour" description="Replay Solar’s origin and revisit setup. Your existing profiles and connections stay in place.">
                    <button className="button button-secondary" onClick={() => setReplayWelcome(true)}><Sparkles size={14} /> Replay welcome tour</button>
                  </SettingsRow>
                </section>
                <section className="settings-section">
                  <h2>Work and connections</h2>
                  <SettingsRow
                    title="Concurrent model turns"
                    description="Limit how many Orbits can make model requests at once."
                  >
                    <select
                      aria-label="Concurrent model turns"
                      value={Number(data.settings.maxConcurrent ?? 2)}
                      onChange={(event) =>
                        void action("save_settings", {
                          settings: {
                            ...data.settings,
                            maxConcurrent: Number(event.target.value),
                          },
                        })
                      }
                    >
                      {[1, 2, 3, 4].map((count) => (
                        <option key={count}>{count}</option>
                      ))}
                    </select>
                  </SettingsRow>
                  <SettingsRow
                    title="Experimental Claude subscriptions"
                    description="Enable unsupported native subscription access with an explicit warning."
                  >
                    <button
                      className={`switch ${data.settings.experimentalClaude ? "on" : ""}`}
                      role="switch"
                      aria-checked={Boolean(data.settings.experimentalClaude)}
                      aria-label="Enable experimental Claude subscriptions"
                      onClick={() => {
                        if (data.settings.experimentalClaude)
                          void action("save_settings", {
                            settings: {
                              ...data.settings,
                              experimentalClaude: false,
                            },
                          });
                        else
                          setConfirm({
                            title: "Enable experimental Claude subscriptions?",
                            description:
                              "Anthropic does not support third-party subscription use. Access may stop working or accounts may be restricted. Enable only if you accept this limitation.",
                            action: () =>
                              api("save_settings", {
                                settings: {
                                  ...data.settings,
                                  experimentalClaude: true,
                                },
                              }),
                          });
                      }}
                    >
                      <span />
                    </button>
                  </SettingsRow>
                </section>
                <section className="settings-section">
                  <h2>About Solar</h2>
                  <SettingsRow
                    title="Solar 0.1.0"
                    description="A local home for your personal team of Orbits."
                  >
                    <span className="value-label">AGPL-3.0-only</span>
                  </SettingsRow>
                  <SettingsRow
                    title="Your data"
                    description="Profiles, conversations, projects, and memory are stored locally. Model requests go to the connection you select."
                  >
                    <LockKeyhole size={18} />
                  </SettingsRow>
                </section>
                <ResetSolar onReset={async () => {
                  await api("reset_solar", { confirmed: true });
                  if (!native) {
                    setConversationId(null);
                    setDialog(null);
                    setPage("orbits");
                    await refresh();
                    setReplayWelcome(true);
                  }
                }} />
              </div>
            )}
          </>
        )}
      </main>
      <input
        ref={importFile}
        type="file"
        accept="application/json,.json"
        hidden
        onChange={(event) => {
          void importOrbit(event.target.files?.[0]);
          event.target.value = "";
        }}
      />
      {toast && (
        <div className="toast" role="status">
          <Check size={16} />
          {toast}
          <button
            aria-label="Dismiss notification"
            onClick={() => setToast("")}
          >
            <X size={14} />
          </button>
        </div>
      )}
      {dialog?.kind === "orbit" && (
        <OrbitForm
          data={data}
          existing={dialog.value}
          onClose={() => setDialog(null)}
          onSaved={saved}
        />
      )}
      {dialog?.kind === "orbit-settings" && (() => {
        const orbit = data.orbits.find((item) => item.id === dialog.orbitId);
        return orbit ? <OrbitSettingsPanel orbit={orbit} data={data}
          onClose={() => setDialog(null)} onChanged={(change) => { if (change !== "edit") setDialog(null); void refresh(); }} /> : null;
      })()}
      {dialog?.kind === "provider" && (
        <ProviderForm
          existing={dialog.value}
          settings={data.settings}
          onClose={() => setDialog(null)}
          onSaved={saved}
        />
      )}
      {dialog?.kind === "project" && (
        <ProjectForm
          existing={dialog.value}
          onClose={() => setDialog(null)}
          onSaved={saved}
        />
      )}
      {dialog?.kind === "routine" && (
        <RoutineForm
          data={data}
          existing={dialog.value}
          onClose={() => setDialog(null)}
          onSaved={saved}
        />
      )}
      {dialog?.kind === "mcp" && (
        <McpForm
          existing={dialog.value}
          onClose={() => setDialog(null)}
          onSaved={saved}
        />
      )}
      {dialog?.kind === "memory" && (
        <MemoryForm
          data={data}
          existing={dialog.value}
          onClose={() => setDialog(null)}
          onSaved={saved}
        />
      )}
      {dialog?.kind === "group" && (
        <GroupForm
          data={data}
          onClose={() => setDialog(null)}
          onCreated={(conversation) => {
            setDialog(null);
            setConversationId(conversation.id);
            setPage("conversations");
            void refresh();
          }}
        />
      )}
      {dialog?.kind === "export" && (
        <Modal
          title="Portable Orbit profile"
          subtitle="This profile contains its personality and configuration. Review it before sharing."
          onClose={() => setDialog(null)}
        >
          <div className="form-body">
            <textarea
              className="export-json"
              rows={15}
              readOnly
              value={dialog.json}
            />
            <div className="button-row">
              <button
                className="button button-primary"
                onClick={() => {
                  void navigator.clipboard
                    .writeText(dialog.json)
                    .then(() => setToast("Profile copied."))
                    .catch((failure) => setError(errorText(failure)));
                }}
              >
                Copy profile
              </button>
              <button
                className="button button-secondary"
                onClick={() => {
                  const url = URL.createObjectURL(
                    new Blob([dialog.json], { type: "application/json" }),
                  );
                  const link = document.createElement("a");
                  link.href = url;
                  link.download = "orbit.solar.json";
                  link.click();
                  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
                }}
              >
                <Download size={15} />
                Download
              </button>
            </div>
          </div>
        </Modal>
      )}
      {dialog?.kind === "help" && (
        <Modal
          title="Welcome to Solar"
          subtitle="Your Orbits live here, and work with you on your computer."
          onClose={() => setDialog(null)}
        >
          <div className="form-body help-body">
            <Planet sun color="#e8aa62" size={104} />
            <p>
              Start by connecting a model. Create an Orbit, choose its specialty
              and tools, and open a conversation. Shared projects let your
              Orbits work together while keeping their private memories
              separate.
            </p>
            <p>
              Hover over the desktop sun to see orbital paths. Click to bring
              your Orbits out, drag them to a comfortable spot, and click an
              Orbit to chat.
            </p>
            {!native && (
              <div className="notice">
                This browser is a UI preview. Model calls, screen sharing, and
                native tools run in the desktop application.
              </div>
            )}
          </div>
        </Modal>
      )}
      {confirm && (
        <Modal title={confirm.title} onClose={() => setConfirm(null)}>
          <div className="form-body">
            <p className="confirm-description">{confirm.description}</p>
            <div className="button-row justify-end">
              <button
                className="button button-quiet"
                onClick={() => setConfirm(null)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  void confirm
                    .action()
                    .then(async () => {
                      setConfirm(null);
                      await refresh();
                    })
                    .catch((failure) => setError(errorText(failure)))
                    .finally(() => setBusy(false));
                }}
              >
                {busy && <LoaderCircle size={15} className="spin" />}Confirm
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

function SettingsRow({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-row">
      <div>
        <h3>{title}</h3>
        <p>{description}</p>
      </div>
      {children}
    </div>
  );
}
