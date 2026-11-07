import { useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import {
  ArrowLeft, ArrowRight, Check, ChevronRight, Folder, Globe2, Heart,
  Layers3, LoaderCircle, LockKeyhole, MessageCircle, Plus, Settings2,
  ShieldCheck, Sparkles, Sun, Users, Volume2, VolumeX,
} from "lucide-react";
import { api, native } from "../api";
import { errorText, isProviderConfigured } from "../types";
import type { Bootstrap, Orbit, Project, Provider, Settings } from "../types";
import { OrbitForm, ProjectForm, ProviderForm } from "../components/Forms";
import { Planet } from "../components/Planet";
import { StarBirth } from "./StarBirth";
import "./onboarding.css";

const steps = ["birth", "welcome", "team", "desktop", "connection", "orbit", "workspace", "ready"] as const;
type Step = typeof steps[number];
type Editor = { kind: "provider"; value?: Provider } | { kind: "orbit"; value?: Orbit } | { kind: "project"; value?: Project };
const labels: Record<Step, string> = {
  birth: "A star is born", welcome: "Meet your Orbits", team: "How you work together",
  desktop: "At home on your desktop", connection: "Choose a connection", orbit: "Make an Orbit yours",
  workspace: "Give your work a home", ready: "Ready when you are",
};

function validStep(value: unknown): Step {
  return steps.includes(value as Step) ? value as Step : "birth";
}

function Later({ children }: { children: ReactNode }) {
  return <p className="onboarding-later"><Settings2 size={13} />{children}</p>;
}

function SettingSwitch({ title, description, checked, disabled, onChange }: {
  title: string; description: string; checked: boolean; disabled: boolean; onChange: () => void;
}) {
  return <div className="onboarding-setting">
    <div><strong>{title}</strong><p>{description}</p></div>
    <button type="button" className={`switch ${checked ? "on" : ""}`} role="switch" aria-label={title} aria-checked={checked} disabled={disabled} onClick={onChange}><span /></button>
  </div>;
}

export function Onboarding({ data, onComplete, refresh, initialStep, onNavigate }: {
  data: Bootstrap;
  onComplete: () => void;
  refresh: () => Promise<void>;
  initialStep?: string;
  onNavigate?: (page: "orbits" | "connections" | "projects" | "settings") => void;
}) {
  const [step, setStep] = useState<Step>(() => validStep(initialStep ?? data.settings.onboardingStep));
  const [editor, setEditor] = useState<Editor | null>(null);
  const [selectedOrbitId, setSelectedOrbitId] = useState(data.orbits.find((orbit) => orbit.enabled)?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [musicPlaying, setMusicPlaying] = useState(false);
  const [musicUnavailable, setMusicUnavailable] = useState(false);
  const [musicBusy, setMusicBusy] = useState(false);
  const [desktopDemoExpanded, setDesktopDemoExpanded] = useState(false);
  const [loginPending, setLoginPending] = useState<Record<string, boolean>>({});
  const [loginMessage, setLoginMessage] = useState<Record<string, string>>({});
  const heading = useRef<HTMLHeadingElement>(null);
  const musicSequence = useRef(0);
  const active = useRef(true);
  const settingsRef = useRef(data.settings);
  settingsRef.current = data.settings;
  const reducedMotion = Boolean(data.settings.reducedMotion);
  const index = steps.indexOf(step);
  const enabledOrbits = data.orbits.filter((orbit) => orbit.enabled);
  const selectedOrbit = enabledOrbits.find((orbit) => orbit.id === selectedOrbitId) ?? enabledOrbits[0];
  const pendingIds = Object.keys(loginPending).filter((id) => loginPending[id]).sort().join(",");

  async function setMusic(playing: boolean) {
    const sequence = ++musicSequence.current;
    const response = await api<{ playing: boolean }>("set_onboarding_music", { playing });
    if (active.current && sequence === musicSequence.current) {
      setMusicPlaying(response.playing);
      setMusicUnavailable(playing && !response.playing);
    }
  }

  useEffect(() => {
    active.current = true;
    const visibility = () => {
      const playing = native && document.visibilityState === "visible" && settingsRef.current.onboardingMusic !== false;
      void setMusic(playing).catch(() => { if (active.current) setMusicPlaying(false); });
    };
    document.addEventListener("visibilitychange", visibility);
    if (native) visibility();
    return () => {
      active.current = false;
      musicSequence.current += 1;
      document.removeEventListener("visibilitychange", visibility);
      void api("set_onboarding_music", { playing: false }).catch(() => {});
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.motion = reducedMotion ? "reduced" : "full";
  }, [reducedMotion]);

  useEffect(() => {
    if (step !== "birth") heading.current?.focus({ preventScroll: true });
  }, [step]);

  useEffect(() => {
    if (!pendingIds) return;
    let disposed = false;
    const poll = () => {
      for (const providerId of pendingIds.split(",")) {
        void api<{ connected: boolean; pending: boolean; message?: string }>("login_status", { providerId }).then((status) => {
          if (disposed) return;
          if (!status.pending || status.connected) {
            setLoginPending((current) => ({ ...current, [providerId]: false }));
            setLoginMessage((current) => ({ ...current, [providerId]: status.connected ? "Connected. You can choose a model in the next step." : status.message || "Sign-in ended. You can try again or continue later." }));
            if (status.connected) void refresh();
          }
        }).catch((reason) => {
          if (!disposed) {
            setLoginPending((current) => ({ ...current, [providerId]: false }));
            setLoginMessage((current) => ({ ...current, [providerId]: errorText(reason) }));
          }
        });
      }
    };
    const timer = window.setInterval(poll, 1500);
    poll();
    return () => { disposed = true; window.clearInterval(timer); };
  }, [pendingIds, refresh]);

  async function go(next: Step) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("save_onboarding_progress", { step: next });
      setStep(next);
    } catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }

  async function finish(destination: "orbits" | "connections" = "orbits") {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("set_onboarding_music", { playing: false }).catch(() => {});
      await api("complete_onboarding");
      await refresh();
      onNavigate?.(destination);
      onComplete();
    } catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }

  async function toggleMusic() {
    if (musicBusy) return;
    const next = !musicPlaying;
    setMusicBusy(true);
    setError("");
    try {
      await api("save_settings", { settings: { ...settingsRef.current, onboardingMusic: next } });
      await refresh();
      await setMusic(next);
    } catch (reason) { setError(errorText(reason)); }
    finally { setMusicBusy(false); }
  }

  async function updateSettings(patch: Partial<Settings>) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("save_settings", { settings: { ...settingsRef.current, ...patch } });
      await refresh();
      if (patch.onboardingVolume !== undefined && musicPlaying) await setMusic(true);
    } catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }

  async function startLogin(provider: Provider) {
    setLoginPending((current) => ({ ...current, [provider.id]: true }));
    setLoginMessage((current) => ({ ...current, [provider.id]: "Finish sign-in in the browser, then return here." }));
    try { await api("start_login", { providerId: provider.id }); }
    catch (reason) {
      setLoginPending((current) => ({ ...current, [provider.id]: false }));
      setLoginMessage((current) => ({ ...current, [provider.id]: errorText(reason) }));
    }
  }

  async function discover(provider: Provider) {
    setBusy(true);
    setError("");
    try { await api("discover_models", { id: provider.id }); await refresh(); }
    catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }

  async function saved() {
    setEditor(null);
    await refresh();
  }

  function editFirstOrbit() {
    const draft = selectedOrbit && !selectedOrbit.providerId && !selectedOrbit.model
      ? { ...selectedOrbit, autonomy: "ask" as const }
      : selectedOrbit;
    setEditor({ kind: "orbit", value: draft });
  }

  const title = labels[step];
  const next = steps[Math.min(index + 1, steps.length - 1)];
  const setupStep = ["connection", "orbit", "workspace"].includes(step);

  return <div className={`onboarding ${reducedMotion ? "onboarding-reduced" : ""} ${step === "birth" ? "onboarding-birth" : ""}`}>
    <header className="onboarding-topbar">
      <div className="onboarding-wordmark"><Sun size={20} /><span>solar.</span><span className="onboarding-label">A little universe of help.</span></div>
      <div className="onboarding-top-actions">
        <button type="button" className={`onboarding-music ${musicPlaying ? "playing" : ""}`} aria-label={musicPlaying ? "Mute welcome music" : "Play welcome music"} aria-pressed={musicPlaying} disabled={musicBusy} onClick={() => void toggleMusic()} title={musicUnavailable ? "Music playback is available in the native Mac app" : musicPlaying ? "Mute welcome music" : "Play welcome music"}>
          {musicPlaying ? <Volume2 size={15} /> : <VolumeX size={15} />}
          <span>{musicPlaying ? "Music on" : "Music off"}</span>
        </button>
        {step !== "birth" && step !== "ready" && <button type="button" className="onboarding-skip" onClick={() => void go("ready")} disabled={busy}>Skip setup <ChevronRight size={13} /></button>}
      </div>
    </header>

    {step === "birth" ? <>
      <StarBirth reducedMotion={reducedMotion} onComplete={() => void go("welcome")} />
      {(error || musicUnavailable) && <div className="onboarding-birth-notice" role={error ? "alert" : "status"}>{error || "The preview is silent. Music plays in the native Mac app."}</div>}
    </> : <>
      <div className="onboarding-progress" aria-label={`Welcome step ${index} of ${steps.length - 1}`}>
        {steps.slice(1).map((value, position) => <div key={value} className={`${position + 1 < index ? "done" : ""} ${value === step ? "current" : ""}`}><span /><small>{position + 1 <= 3 ? "Discover" : position + 1 === 7 ? "Launch" : "Set up"}</small></div>)}
      </div>
      <main key={step} className="onboarding-main">
        <div className="onboarding-copy">
          <span className="onboarding-eyebrow">{setupStep ? "MAKE YOURSELF AT HOME" : step === "ready" ? "YOUR UNIVERSE IS READY" : "WELCOME TO SOLAR"} <span>0{index}</span></span>
          <h1 ref={heading} tabIndex={-1}>{title}</h1>
          {step === "welcome" && <>
            <p className="onboarding-lead">A team of thoughtful specialists.<br />A place for each of them in your day.</p>
            <p>Each Orbit has a name, a personality, and one or two things it does especially well. Talk to one on its own, or bring a few together.</p>
            <div className="onboarding-note"><Heart size={17} /><span>More than a role. A teammate you make your own.</span></div>
            <Later>Rename and shape them any time in <strong>Your Orbits</strong>.</Later>
          </>}
          {step === "team" && <>
            <p className="onboarding-lead">Different perspectives.<br />One shared direction.</p>
            <p>Choose a lead for a group conversation. It can ask the other Orbits for help and bring their work back to you.</p>
            <ul className="onboarding-feature-list">
              <li><LockKeyhole size={17} /><span><strong>Private by default.</strong> Each Orbit has its own workspace and memory.</span></li>
              <li><Folder size={17} /><span><strong>Shared when you choose.</strong> Give selected Orbits access to a project.</span></li>
              <li><ShieldCheck size={17} /><span><strong>You set the limits.</strong> Tools and approvals control what they may do.</span></li>
            </ul>
            <Later>Groups live in <strong>Conversations</strong>. Permissions live in each Orbit’s profile.</Later>
          </>}
          {step === "desktop" && <>
            <p className="onboarding-lead">Close the launcher.<br />Keep your team nearby.</p>
            <p>The sun stays on your desktop while Solar runs. Click it to let your Orbits out, drag them to a comfortable spot, and click one for a quick chat.</p>
            <div className="onboarding-note"><Sparkles size={17} /><span>When they need you, they peek down from the top of your screen with their own little chime.</span></div>
            <Later>Change companions, chimes, and motion in <strong>Settings</strong>. Quit Solar from its menu-bar menu.</Later>
          </>}
          {step === "connection" && <>
            <p className="onboarding-lead">First, a place to think.</p>
            <p>Connect your model provider or a local endpoint. You can use different connections and models for different Orbits.</p>
            <div className="onboarding-note"><LockKeyhole size={17} /><span>On Mac, credentials stay in Keychain. Saving a connection does not start a model request.</span></div>
            <Later>Add, sign in, and discover models later in <strong>Connections</strong>.</Later>
          </>}
          {step === "orbit" && <>
            <p className="onboarding-lead">A familiar face.<br />Your kind of help.</p>
            <p>Choose a starter and make it yours: name, specialties, personality, model, and what it can do. Start with Ask first if you want to review every tool action.</p>
            <Later>Edit profiles, add teammates, and choose their tools in <strong>Your Orbits</strong>.</Later>
          </>}
          {step === "workspace" && <>
            <p className="onboarding-lead">Make room for<br />something meaningful.</p>
            <p>A project connects a folder with useful context. Only Orbits you explicitly give access can use its files and shared knowledge.</p>
            <div className="onboarding-note"><ShieldCheck size={17} /><span>Creating a project does not grant every Orbit access. Choose members in their profiles.</span></div>
            <Later>Manage folders and context in <strong>Projects</strong>, and access in <strong>Your Orbits</strong>.</Later>
          </>}
          {step === "ready" && <>
            <p className="onboarding-lead">Start small.<br />Let good things grow.</p>
            <p>Try a question, make a plan, or ask an Orbit to help with a project. You can return to this welcome from Settings whenever you like.</p>
            <div className="onboarding-summary">
              <span><Check size={14} />{enabledOrbits.length} {enabledOrbits.length === 1 ? "Orbit" : "Orbits"} ready to meet</span>
              <span><Globe2 size={14} />{data.providers.length ? `${data.providers.length} ${data.providers.length === 1 ? "connection" : "connections"} saved` : "Model connection can be added later"}</span>
              <span><Folder size={14} />{data.projects.length ? `${data.projects.length} ${data.projects.length === 1 ? "project" : "projects"} saved` : "Projects are optional"}</span>
            </div>
            <Later>Nothing is final. Your Orbits, connections, and preferences are always yours to change.</Later>
          </>}
        </div>

        <div className={`onboarding-visual onboarding-visual-${step}`}>
          {step === "welcome" && <div className="onboarding-family" aria-label="A few of the Orbits you can shape">
            <div className="onboarding-family-orbit family-one"><Planet color="#edb56d" name="Your assistant" size={125} /><strong>Your assistant</strong><small>Thoughtful. Organized.</small></div>
            <div className="onboarding-family-orbit family-two"><Planet color="#90d9c8" name="Your researcher" size={100} /><strong>Your researcher</strong><small>Curious. Clear.</small></div>
            <div className="onboarding-family-orbit family-three"><Planet color="#b9a8e9" name="Your QA specialist" size={88} /><strong>Your QA specialist</strong><small>Observant. Thorough.</small></div>
            <span className="onboarding-orbit-ring ring-one" /><span className="onboarding-orbit-ring ring-two" />
          </div>}
          {step === "team" && <div className="onboarding-team-demo">
            <div className="onboarding-demo-question"><MessageCircle size={16} />“Help me make this launch a good one.”</div>
            <div className="onboarding-team-members">
              {[["#edb56d", "Plan", "Your lead"], ["#90d9c8", "Research", "Evidence"], ["#b9a8e9", "Review", "Quality"]].map(([color, name, role], position) => <div className={`onboarding-team-member member-${position}`} key={name}><Planet color={color} size={66} name={name} /><strong>{name}</strong><small>{role}</small></div>)}
            </div>
            <div className="onboarding-demo-result"><Layers3 size={17} /><span>A clear plan, checked from a few angles.</span><Check size={15} /></div>
            <span className="onboarding-demo-caption">An illustration of how your team works together</span>
          </div>}
          {step === "desktop" && <div className={`onboarding-desktop-demo ${desktopDemoExpanded ? "expanded" : ""}`}>
            <div className="onboarding-desktop-menu"><span /><span /><span /><i>YOUR DESKTOP</i></div>
            <div className="onboarding-peek-demo"><Planet color="#90d9c8" name="An Orbit asking for attention" size={38} /><span>A little help?</span></div>
            <button type="button" className="onboarding-sun-demo" aria-label={desktopDemoExpanded ? "Gather the example Orbits" : "Expand the example Orbits"} aria-expanded={desktopDemoExpanded} onClick={() => setDesktopDemoExpanded((current) => !current)}><Planet sun color="#edb56d" name="Solar" size={87} /></button>
            {["#edb56d", "#90d9c8", "#b9a8e9"].map((color, position) => <span className={`onboarding-desktop-orbit demo-orbit-${position}`} key={color}><Planet color={color} name="Example Orbit" size={53} /></span>)}
            <span className="onboarding-desktop-hint">Click the sun. Try it.</span>
          </div>}
          {step === "connection" && <div className="onboarding-setup-card">
            <div className="onboarding-card-heading"><Globe2 size={19} /><h2>Model connections</h2></div>
            {data.providers.length ? <div className="onboarding-connection-list">{data.providers.map((provider) => <article className="onboarding-connection" key={provider.id}>
              <div><strong>{provider.name}</strong><span className={isProviderConfigured(provider) ? "is-ready" : ""}>{provider.connected ? "Connected" : isProviderConfigured(provider) ? "Configured" : "Needs sign-in"}</span></div>
              <p>{["chatgpt", "claude"].includes(provider.kind) ? provider.kind === "claude" ? "Claude subscription · experimental" : "ChatGPT subscription" : provider.baseUrl}</p>
              <div className="onboarding-small-actions">
                {["chatgpt", "claude"].includes(provider.kind) && !provider.connected && <button type="button" className="button button-primary button-small" disabled={loginPending[provider.id]} onClick={() => void startLogin(provider)}>{loginPending[provider.id] ? <><LoaderCircle size={12} className="spin" />Signing in…</> : "Sign in"}</button>}
                <button type="button" className="button button-secondary button-small" disabled={busy} onClick={() => void discover(provider)}>Discover models</button>
                <button type="button" className="icon-button" aria-label={`Edit ${provider.name}`} onClick={() => setEditor({ kind: "provider", value: provider })}><Settings2 size={14} /></button>
              </div>
              {loginMessage[provider.id] && <small role="status">{loginMessage[provider.id]}</small>}
            </article>)}</div> : <div className="onboarding-card-empty"><Sparkles size={29} /><h3>Choose a model connection</h3><p>A local endpoint, an API provider, or subscription sign-in.</p></div>}
            <button type="button" className="button button-primary" onClick={() => setEditor({ kind: "provider" })}><Plus size={15} />Add a connection</button>
          </div>}
          {step === "orbit" && <div className="onboarding-setup-card onboarding-orbit-setup">
            {selectedOrbit ? <>
              <div className="onboarding-orbit-showcase" style={{ "--onboarding-orbit-color": selectedOrbit.color } as CSSProperties}><Planet color={selectedOrbit.color} name={selectedOrbit.name} size={114} /></div>
              <h2>{selectedOrbit.name}</h2><p>{selectedOrbit.role}</p>
              <label className="field"><span>Start with an Orbit</span><select aria-label="Choose your first Orbit" value={selectedOrbit.id} onChange={(event) => setSelectedOrbitId(event.target.value)}>{enabledOrbits.map((orbit) => <option value={orbit.id} key={orbit.id}>{orbit.name} · {orbit.role}</option>)}</select></label>
              <button type="button" className="button button-primary" onClick={editFirstOrbit}><Settings2 size={15} />Shape this Orbit</button>
              <small>{selectedOrbit.providerId && selectedOrbit.model ? `Assigned model: ${selectedOrbit.model}` : "Choose a connection and model in their profile."}</small>
            </> : <><Planet color="#edb56d" name="A new Orbit" size={100} /><h2>A teammate of your own</h2><button type="button" className="button button-primary" onClick={() => setEditor({ kind: "orbit" })}><Plus size={15} />Create an Orbit</button></>}
          </div>}
          {step === "workspace" && <div className="onboarding-setup-card">
            <div className="onboarding-card-heading"><Folder size={19} /><h2>Your projects</h2></div>
            {data.projects.length ? <div className="onboarding-project-list">{data.projects.map((project) => <button type="button" key={project.id} onClick={() => setEditor({ kind: "project", value: project })}><Folder size={18} /><span><strong>{project.name}</strong><small>{project.directory}</small></span><ChevronRight size={14} /></button>)}</div> : <div className="onboarding-card-empty"><Folder size={30} /><h3>Pick a folder. Add some context.</h3><p>A project is optional. Every Orbit already has a private workspace.</p></div>}
            <button type="button" className="button button-primary" onClick={() => setEditor({ kind: "project" })}><Plus size={15} />Add a project</button>
            {selectedOrbit && data.projects.length > 0 && <button type="button" className="button button-secondary" onClick={editFirstOrbit}><Users size={15} />Choose {selectedOrbit.name}’s access</button>}
          </div>}
          {step === "ready" && <div className="onboarding-setup-card onboarding-preferences">
            <div className="onboarding-ready-sun"><Planet sun color="#edb56d" name="Solar" size={104} /></div>
            <SettingSwitch title="Desktop companions" description="Keep the sun nearby while Solar runs." checked={data.settings.desktopEnabled !== false} disabled={busy} onChange={() => void updateSettings({ desktopEnabled: data.settings.desktopEnabled === false })} />
            <SettingSwitch title="Orbit chimes" description="A short melody when a collapsed Orbit needs you." checked={data.settings.attentionSound !== false} disabled={busy} onChange={() => void updateSettings({ attentionSound: data.settings.attentionSound === false })} />
            <SettingSwitch title="Reduced motion" description="Use still illustrations and gentle transitions." checked={reducedMotion} disabled={busy} onChange={() => void updateSettings({ reducedMotion: !reducedMotion })} />
            <div className="onboarding-setting"><div><strong>Welcome music</strong><p>A little atmosphere for this introduction.</p></div><label className="onboarding-volume"><input type="range" min="0" max="0.5" step="0.01" aria-label="Welcome music volume" disabled={busy} value={Number(data.settings.onboardingVolume ?? 0.22)} onChange={(event) => void updateSettings({ onboardingVolume: Number(event.target.value) })} /><output>{Math.round(Number(data.settings.onboardingVolume ?? 0.22) * 200)}%</output></label></div>
            <p className="onboarding-card-footnote">You’ll find all of these again in Settings.</p>
          </div>}
        </div>
      </main>

      <footer className="onboarding-footer">
        <div className="onboarding-footer-left"><button type="button" className="button button-quiet" disabled={busy} onClick={() => void go(steps[Math.max(0, index - 1)])}><ArrowLeft size={15} />Back</button><span>{index} / {steps.length - 1}</span></div>
        <div className="onboarding-footer-status">{error ? <span role="alert">{error}</span> : musicUnavailable ? <span>Music plays in the native Mac app.</span> : <span>Your setup is saved as you go.</span>}</div>
        <div className="onboarding-footer-right">
          {setupStep && <button type="button" className="button button-quiet" disabled={busy} onClick={() => void go(next)}>Do this later</button>}
          <button type="button" className="button button-primary" disabled={busy} onClick={() => step === "ready" ? void finish() : void go(next)}>{busy ? <LoaderCircle size={15} className="spin" /> : null}{step === "ready" ? "Open Solar" : step === "desktop" ? "Make it yours" : "Continue"}<ArrowRight size={15} /></button>
        </div>
      </footer>
    </>}
    {editor?.kind === "provider" && <ProviderForm existing={editor.value} settings={data.settings} onClose={() => setEditor(null)} onSaved={() => void saved()} />}
    {editor?.kind === "orbit" && <OrbitForm existing={editor.value} data={data} onClose={() => setEditor(null)} onSaved={() => void saved()} />}
    {editor?.kind === "project" && <ProjectForm existing={editor.value} onClose={() => setEditor(null)} onSaved={() => void saved()} />}
  </div>;
}
