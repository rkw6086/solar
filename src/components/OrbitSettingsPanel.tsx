import { useEffect, useRef, useState } from "react";
import { Check, Copy, LoaderCircle, RotateCcw, Settings2, Terminal, Trash2 } from "lucide-react";
import { api, native } from "../api";
import { errorText } from "../types";
import type { Bootstrap, Orbit } from "../types";
import { Modal } from "./Modal";
import { Planet } from "./Planet";
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
    subtitle="Set up command-line tools for this Orbit." onClose={closeCli} dismissible={!cliOpening}>
    <div className="form-body orbit-cli-body">
      <p className="orbit-cli-intro">This Orbit uses its own home and configuration folders. Solar does not automatically copy your personal CLI sign-ins. You can also ask this Orbit in chat to import specific home files or shell profiles; it will show the files for approval first.</p>
      <ol className="orbit-cli-steps">
        <li><strong>Open the Orbit’s terminal.</strong><span>It starts in this Orbit’s workspace, using the same private configuration as its shell commands.</span></li>
        <li><strong>Sign in to the tools it needs.</strong><span>For GitLab, run <code>glab auth login --git-protocol https --insecure-storage</code> and follow the prompts for your GitLab host. This uses HTTPS for Git and keeps the login in the Orbit’s private configuration folder instead of your shared system keychain. Enter credentials in the terminal, not in chat.</span></li>
        <li><strong>Return to Solar.</strong><span>Close the terminal when you’re finished, then ask {orbit.name} to try again. Each Orbit signs in separately.</span></li>
      </ol>
      {cliLoading && <p className="orbit-cli-loading" role="status"><LoaderCircle size={16} className="spin" />Preparing this Orbit’s terminal…</p>}
      {cliError && <p className="form-error" role="alert">{cliError}</p>}
      {cliOpened && <p className="orbit-cli-opened" role="status"><Check size={16} />Terminal opened. Complete the sign-in there, then return to Solar.</p>}
      {!native && <p className="orbit-cli-preview">This is a browser preview. Open Solar’s desktop app to prepare an actual Orbit terminal.</p>}
      {cliSetup && <details className="orbit-cli-details">
        <summary>Folders and manual setup</summary>
        <dl className="orbit-settings-summary">
          <div><dt>Shell</dt><dd>{cliSetup.shell}</dd></div>
          <div><dt>Home and workspace</dt><dd>{cliSetup.home}</dd></div>
          <div><dt>Private configuration</dt><dd>{cliSetup.configDirectory}</dd></div>
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
        {cliOpening ? <LoaderCircle size={16} className="spin" /> : <Terminal size={16} />}{cliOpened ? "Open terminal again" : "Open Orbit terminal"}
      </button>
    </footer>
  </Modal>;
  if (confirm) return <Modal title={confirm === "reset" ? `Reset ${orbit.name}?` : `Delete ${orbit.name}?`}
    onClose={() => { if (!busy) { setConfirm(null); setError(""); } }}>
    <div className="form-body">
      <p className="orbit-settings-confirm-copy">{confirm === "reset"
        ? "This stops current work and clears this Orbit’s private conversations, saved memory, and activity. Its profile, model, project access, workspace files, and shared group conversations stay in place."
        : "This stops current work and removes this Orbit from your team. Its profile will no longer be available in Solar. Your project files are kept."}</p>
      {error && <p className="form-error" role="alert">{error}</p>}
    </div>
    <footer className="form-footer">
      <button type="button" className="button button-quiet" disabled={busy} onClick={() => { setConfirm(null); setError(""); }}>Cancel</button>
      <button type="button" className="button orbit-settings-danger" disabled={busy} onClick={() => void perform()}>
        {busy && <LoaderCircle size={15} className="spin" />}{confirm === "reset" ? "Reset Orbit" : "Delete Orbit"}
      </button>
    </footer>
  </Modal>;
  return <Modal title={`${orbit.name} settings`} subtitle="Manage this Orbit’s character, model, and saved data." onClose={onClose}>
    <div className="form-body orbit-settings-body">
      <div className="orbit-settings-identity"><Planet identity={orbit.id} name={orbit.name} color={orbit.color} size={76} /><div><h3>{orbit.name}</h3><p>{orbit.role}</p></div></div>
      <dl className="orbit-settings-summary">
        <div><dt>Model</dt><dd>{orbit.model ? `${orbit.model} (${provider?.name ?? "Unknown provider"})` : "No model selected"}</dd></div>
        <div><dt>Personality</dt><dd>{orbit.personality || "Not set"}</dd></div>
        <div><dt>Tasks</dt><dd>{orbit.tasks || "Not set"}</dd></div>
      </dl>
      <button type="button" className="button button-primary" onClick={() => setEditing(true)}><Settings2 size={16} />Edit Orbit</button>
      <div className="orbit-settings-data">
        <h3>CLI sign-ins</h3>
        <p>Give this Orbit its own sign-ins for command-line tools such as GitLab.</p>
        <button type="button" className="button button-secondary" onClick={() => void loadCli()}><Terminal size={16} />Set up CLI sign-ins</button>
      </div>
      <div className="orbit-settings-data">
        <h3>Saved data</h3>
        <p>Clear learned context without changing this Orbit’s setup.</p>
        <button type="button" className="button button-secondary" onClick={() => setConfirm("reset")}><RotateCcw size={15} />Reset Orbit</button>
        <button type="button" className="button button-quiet orbit-settings-delete" onClick={() => setConfirm("delete")}><Trash2 size={15} />Delete Orbit</button>
      </div>
    </div>
  </Modal>;
}
