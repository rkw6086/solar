import { useEffect, useRef, useState } from "react";
import { Check, Copy, LoaderCircle, RotateCcw, Settings2, Shuffle, Terminal, Trash2 } from "lucide-react";
import { api, native } from "../api";
import { errorText } from "../types";
import type { Bootstrap, Orbit } from "../types";
import { Modal } from "./Modal";
import { Planet } from "./Planet";
import { generateOrbitAvatarSeed } from "./orbitIdentity";
import { OrbitForm } from "./Forms";
import "./orbitSettings.css";

interface OrbitCliSetup {
  shell: string;
  home: string;
  configDirectory: string;
  command: string;
}

export function OrbitSettingsPanel({ orbit, data, onClose, onChanged }: {
  orbit: Orbit;
  data: Bootstrap;
  onClose: () => void;
  onChanged: (change: "edit" | "reset" | "delete") => void;
}) {
  const [editing, setEditing] = useState(false);
  const [avatarSeed, setAvatarSeed] = useState(orbit.avatarSeed);
  const [avatarBusy, setAvatarBusy] = useState(false);
  const [avatarError, setAvatarError] = useState("");
  const [avatarUpdated, setAvatarUpdated] = useState(false);
  const avatarRequest = useRef(false);
  useEffect(() => setAvatarSeed(orbit.avatarSeed), [orbit.avatarSeed]);
  async function regenerateAvatar() {
    if (avatarRequest.current) return;
    avatarRequest.current = true;
    setAvatarBusy(true);
    setAvatarError("");
    setAvatarUpdated(false);
    try {
      const nextSeed = generateOrbitAvatarSeed(avatarSeed || orbit.id, orbit.color);
      const updated = await api<Orbit>("set_orbit_avatar", { orbitId: orbit.id, avatarSeed: nextSeed });
      setAvatarSeed(updated.avatarSeed);
      setAvatarUpdated(true);
      onChanged("edit");
    } catch (reason) {
      setAvatarError(errorText(reason));
    } finally {
      avatarRequest.current = false;
      setAvatarBusy(false);
    }
  }
  const [confirm, setConfirm] = useState<"reset" | "delete" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [cliVisible, setCliVisible] = useState(false);
  const [cliSetup, setCliSetup] = useState<OrbitCliSetup | null>(null);
  const [cliLoading, setCliLoading] = useState(false);
  const [cliOpening, setCliOpening] = useState(false);
  const [cliOpened, setCliOpened] = useState(false);
  const [cliCopied, setCliCopied] = useState(false);
  const [cliError, setCliError] = useState("");
  const cliRequest = useRef(0);
  const cliOpeningRef = useRef(false);
  useEffect(() => () => { cliRequest.current += 1; }, []);
  const provider = data.providers.find((item) => item.id === orbit.providerId);
  function closeCli() {
    if (cliOpeningRef.current) return;
    cliRequest.current += 1;
    setCliVisible(false);
  }
  async function loadCli() {
    const request = ++cliRequest.current;
    setCliVisible(true);
    setCliLoading(true);
    setCliSetup(null);
    setCliOpened(false);
    setCliCopied(false);
    setCliError("");
    try {
      const setup = await api<OrbitCliSetup>("get_orbit_cli_setup", { orbitId: orbit.id });
      if (cliRequest.current === request) setCliSetup(setup);
    } catch (reason) {
      if (cliRequest.current === request) setCliError(errorText(reason));
    } finally {
      if (cliRequest.current === request) setCliLoading(false);
    }
  }
  async function openCli() {
    if (cliOpeningRef.current || !cliSetup) return;
    const request = cliRequest.current;
    cliOpeningRef.current = true;
    setCliOpening(true);
    setCliOpened(false);
    setCliError("");
    try {
      await api("open_orbit_cli_setup", { orbitId: orbit.id });
      if (cliRequest.current === request) setCliOpened(true);
    } catch (reason) {
      if (cliRequest.current === request) setCliError(errorText(reason));
    } finally {
      cliOpeningRef.current = false;
      if (cliRequest.current === request) setCliOpening(false);
    }
  }
  async function copyCli() {
    if (!cliSetup) return;
    const request = cliRequest.current;
    setCliError("");
    setCliCopied(false);
    try {
      await navigator.clipboard.writeText(cliSetup.command);
      if (cliRequest.current === request) setCliCopied(true);
    } catch {
      if (cliRequest.current === request) setCliError("Could not copy automatically. Select the setup command below and copy it.");
    }
  }
  async function perform() {
    if (!confirm || busy) return;
    setBusy(true);
    setError("");
    try {
      if (confirm === "reset") await api("reset_orbit", { orbitId: orbit.id, confirmed: true });
      else await api("delete_orbit", { id: orbit.id });
      onChanged(confirm);
      onClose();
    } catch (reason) {
      setError(errorText(reason));
      setBusy(false);
    }
  }
  if (editing) return <OrbitForm data={data} existing={orbit}
    onClose={() => setEditing(false)} onSaved={() => { setEditing(false); onChanged("edit"); }} />;
  if (cliVisible) return <Modal title={`${orbit.name}’s CLI sign-ins`}
    subtitle="Set up command-line tools for this Sprite." onClose={closeCli} dismissible={!cliOpening}>
    <div className="form-body orbit-cli-body">
      <p className="orbit-cli-intro">This Sprite uses its workspace and the configuration you explicitly shared. You can ask it to find and request additional home files or profiles; adding live links always needs your approval.</p>
      <ol className="orbit-cli-steps">
        <li><strong>Open the Sprite’s terminal.</strong><span>It starts in this Sprite’s workspace, using the same private or explicitly shared configuration as its shell commands.</span></li>
        <li><strong>Sign in to the tools it needs.</strong><span>For GitLab, run <code>glab auth login --git-protocol https --insecure-storage</code> and follow the prompts for your GitLab host. This uses HTTPS for Git and stores the login in the Sprite’s selected configuration instead of the system keychain. A writable shared configuration changes the original; a read-only one cannot save a new login. Enter credentials in the terminal, not in chat.</span></li>
        <li><strong>Return to Solar.</strong><span>Close the terminal when you’re finished, then ask {orbit.name} to try again. Sign-ins stay separate unless you explicitly share their configuration.</span></li>
      </ol>
      {cliLoading && <p className="orbit-cli-loading" role="status"><LoaderCircle size={16} className="spin" />Preparing this Sprite’s terminal…</p>}
      {cliError && <p className="form-error" role="alert">{cliError}</p>}
      {cliOpened && <p className="orbit-cli-opened" role="status"><Check size={16} />Terminal opened. Complete the sign-in there, then return to Solar.</p>}
      {!native && <p className="orbit-cli-preview">This is a browser preview. Open Solar’s desktop app to prepare an actual Sprite terminal.</p>}
      {cliSetup && <details className="orbit-cli-details">
        <summary>Folders and manual setup</summary>
        <dl className="orbit-settings-summary">
          <div><dt>Shell</dt><dd>{cliSetup.shell}</dd></div>
          <div><dt>Home and workspace</dt><dd>{cliSetup.home}</dd></div>
          <div><dt>Configuration</dt><dd>{cliSetup.configDirectory}</dd></div>
        </dl>
        <label className="orbit-cli-command-label" htmlFor="orbit-cli-command">{native ? "Run this in a terminal if it does not open automatically." : "Example setup command"}</label>
        <textarea id="orbit-cli-command" className="orbit-cli-command" readOnly value={cliSetup.command} rows={5} spellCheck={false}
          onFocus={(event) => event.currentTarget.select()} />
        <button type="button" className="button button-quiet" onClick={() => void copyCli()}><Copy size={15} />{cliCopied ? "Copied" : "Copy command"}</button>
      </details>}
    </div>
    <footer className="form-footer">
      <button type="button" className="button button-quiet" disabled={cliOpening} onClick={closeCli}>Back to settings</button>
      {!cliSetup && !cliLoading && <button type="button" className="button button-secondary" onClick={() => void loadCli()}>Try again</button>}
      <button type="button" className="button button-primary" disabled={!native || !cliSetup || cliLoading || cliOpening} onClick={() => void openCli()}>
        {cliOpening ? <LoaderCircle size={16} className="spin" /> : <Terminal size={16} />}{cliOpened ? "Open terminal again" : "Open Sprite terminal"}
      </button>
    </footer>
  </Modal>;
  if (confirm) return <Modal title={confirm === "reset" ? `Reset ${orbit.name}?` : `Delete ${orbit.name}?`}
    onClose={() => { if (!busy) { setConfirm(null); setError(""); } }}>
    <div className="form-body">
      <p className="orbit-settings-confirm-copy">{confirm === "reset"
        ? "This stops current work and clears this Sprite’s private conversations, saved memory, and activity. Its profile, model, project access, workspace files, and shared group conversations stay in place."
        : "This stops current work and removes this Sprite from your team. Its profile will no longer be available in Solar. Your project files are kept."}</p>
      {error && <p className="form-error" role="alert">{error}</p>}
    </div>
    <footer className="form-footer">
      <button type="button" className="button button-quiet" disabled={busy} onClick={() => { setConfirm(null); setError(""); }}>Cancel</button>
      <button type="button" className="button orbit-settings-danger" disabled={busy} onClick={() => void perform()}>
        {busy && <LoaderCircle size={15} className="spin" />}{confirm === "reset" ? "Reset Sprite" : "Delete Sprite"}
      </button>
    </footer>
  </Modal>;
  return <Modal title={`${orbit.name} settings`} subtitle="Manage this Sprite’s character, model, and saved data." onClose={onClose}>
    <div className="form-body orbit-settings-body">
      <div className="orbit-settings-identity"><Planet identity={avatarSeed || orbit.id} name={orbit.name} color={orbit.color} size={76} /><div><h3>{orbit.name}</h3><p>{orbit.role}</p>
        <button type="button" className="button button-quiet orbit-settings-avatar" disabled={avatarBusy} onClick={() => void regenerateAvatar()}>
          {avatarBusy ? <LoaderCircle size={14} className="spin" /> : <Shuffle size={14} />}{avatarBusy ? "Updating avatar…" : "Regenerate avatar"}
        </button>
        {avatarUpdated && <span className="orbit-avatar-status" role="status">Avatar updated.</span>}
      </div></div>
      {avatarError && <p className="form-error" role="alert">{avatarError}</p>}
      <dl className="orbit-settings-summary">
        <div><dt>Model</dt><dd>{orbit.model ? `${orbit.model} (${provider?.name ?? "Unknown provider"})` : "No model selected"}</dd></div>
        <div><dt>Personality</dt><dd>{orbit.personality || "Not set"}</dd></div>
        <div><dt>Tasks</dt><dd>{orbit.tasks || "Not set"}</dd></div>
      </dl>
      <button type="button" className="button button-primary" disabled={avatarBusy} onClick={() => setEditing(true)}><Settings2 size={16} />Edit Sprite</button>
      <div className="orbit-settings-data">
        <h3>CLI sign-ins</h3>
        <p>Give this Sprite its own sign-ins for command-line tools such as GitLab.</p>
        <button type="button" className="button button-secondary" onClick={() => void loadCli()}><Terminal size={16} />Set up CLI sign-ins</button>
      </div>
      <div className="orbit-settings-data">
        <h3>Saved data</h3>
        <p>Clear learned context without changing this Sprite’s setup.</p>
        <button type="button" className="button button-secondary" disabled={avatarBusy} onClick={() => setConfirm("reset")}><RotateCcw size={15} />Reset Sprite</button>
        <button type="button" className="button button-quiet orbit-settings-delete" disabled={avatarBusy} onClick={() => setConfirm("delete")}><Trash2 size={15} />Delete Sprite</button>
      </div>
    </div>
  </Modal>;
}
