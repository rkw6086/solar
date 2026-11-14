import { useState } from "react";
import { LoaderCircle, RotateCcw, Settings2, Trash2 } from "lucide-react";
import { api } from "../api";
import { errorText } from "../types";
import type { Bootstrap, Orbit } from "../types";
import { Modal } from "./Modal";
import { Planet } from "./Planet";
import { OrbitForm } from "./Forms";
import "./orbitSettings.css";

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
  const provider = data.providers.find((item) => item.id === orbit.providerId);
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
      <div className="orbit-settings-identity"><Planet name={orbit.name} color={orbit.color} size={76} /><div><h3>{orbit.name}</h3><p>{orbit.role}</p></div></div>
      <dl className="orbit-settings-summary">
        <div><dt>Model</dt><dd>{orbit.model ? `${orbit.model} (${provider?.name ?? "Unknown provider"})` : "No model selected"}</dd></div>
        <div><dt>Personality</dt><dd>{orbit.personality || "Not set"}</dd></div>
        <div><dt>Tasks</dt><dd>{orbit.tasks || orbit.instructions || "Not set"}</dd></div>
      </dl>
      <details className="orbit-settings-prompt"><summary>System prompt</summary><pre>{orbit.instructions || "No system prompt saved."}</pre></details>
      <button type="button" className="button button-primary" onClick={() => setEditing(true)}><Settings2 size={16} />Edit Orbit</button>
      <div className="orbit-settings-data">
        <h3>Saved data</h3>
        <p>Clear learned context without changing this Orbit’s setup.</p>
        <button type="button" className="button button-secondary" onClick={() => setConfirm("reset")}><RotateCcw size={15} />Reset Orbit</button>
        <button type="button" className="button button-quiet orbit-settings-delete" onClick={() => setConfirm("delete")}><Trash2 size={15} />Delete Orbit</button>
      </div>
    </div>
  </Modal>;
}
