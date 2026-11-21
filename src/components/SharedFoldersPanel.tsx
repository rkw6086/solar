import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Check, FolderOpen, Link2, LoaderCircle, LockKeyhole, Plus, RefreshCw, Settings2, Trash2, Users } from "lucide-react";
import { api, native } from "../api";
import { errorText } from "../types";
import type { Bootstrap, HomeShareCandidate, SharedFolder } from "../types";
import { Modal } from "./Modal";
import "./sharedFolders.css";

const samePath = (left: string, right: string) => left.replace(/[/\\]+$/, "") === right.replace(/[/\\]+$/, "");
const folderName = (path: string) => path.split(/[/\\]/).filter(Boolean).at(-1) || "Shared folder";
const defaultMount = (folder: SharedFolder) => `shared/${folder.name.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-|-$/g, "").slice(0, 48) || "folder"}-${folder.id.slice(0, 8)}`;

function SharedFolderForm({ existing, data, onClose, onSaved }: {
  existing?: SharedFolder; data: Bootstrap; onClose: () => void; onSaved: () => Promise<void>;
}) {
  const [folder, setFolder] = useState<SharedFolder>(() => existing ? { ...existing, orbitIds: [...existing.orbitIds] } : {
    id: crypto.randomUUID(), name: "", directory: "", mountPath: "", allSprites: true, orbitIds: [], writable: false,
  });
  const [busy, setBusy] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState("");
  const close = useCallback(() => { if (!busy && !choosing) onClose(); }, [busy, choosing, onClose]);
  const disabled = busy || choosing;
  const suggestedMount = defaultMount({ ...folder, name: folder.name.trim() || folderName(folder.directory) });

  async function chooseFolder() {
    setChoosing(true); setError("");
    try {
      const selected = await api<{ directory: string; name: string } | null>("choose_shared_directory", { directory: folder.directory || null });
      if (selected) setFolder(value => ({ ...value, directory: selected.directory, name: value.name || selected.name }));
    } catch (reason) { setError(errorText(reason)); }
    finally { setChoosing(false); }
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (disabled) return;
    setBusy(true); setError("");
    try {
      const named = { ...folder, name: folder.name.trim() || folderName(folder.directory) };
      await api("save_shared_folder", { folder: { ...named, mountPath: folder.mountPath.trim() || defaultMount(named) } });
      await onSaved(); onClose();
    } catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }
  return <Modal title={existing ? "Edit shared folder" : "Share a folder"}
    subtitle="Create a live link inside the Sprites’ workspaces. Your originals stay where they are."
    onClose={close} dismissible={!disabled}>
    <form onSubmit={event => void submit(event)}>
      <div className="form-body shared-folder-form">
        <label className="field"><span>Folder or file on your computer</span><div className="shared-folder-picker">
          <input required value={folder.directory} disabled={disabled} placeholder="Choose a folder or paste a path"
            onChange={event => setFolder(value => ({ ...value, directory: event.target.value }))} />
          <button type="button" className="button button-secondary" disabled={disabled || !native} onClick={() => void chooseFolder()}>
            {choosing ? <LoaderCircle size={15} className="spin" /> : <FolderOpen size={15} />}Choose folder
          </button>
        </div></label>
        <label className="field"><span>Name <small>optional</small></span><input maxLength={100} value={folder.name} disabled={disabled} placeholder={folder.directory ? folderName(folder.directory) : "Named after the folder"}
          onChange={event => setFolder(value => ({ ...value, name: event.target.value }))} /></label>
        <fieldset className="shared-folder-scope" disabled={disabled}>
          <legend>Who can use it?</legend>
          <label><input type="radio" name="sharing-scope" checked={folder.allSprites} onChange={() => setFolder(value => ({ ...value, allSprites: true }))} /><span>All Sprites <small>Includes Sprites you create later.</small></span></label>
          <label><input type="radio" name="sharing-scope" checked={!folder.allSprites} onChange={() => setFolder(value => ({ ...value, allSprites: false }))} /><span>Choose Sprites</span></label>
          {!folder.allSprites && <div className="shared-folder-members">
            {data.orbits.length ? data.orbits.map(orbit => <label key={orbit.id}><input type="checkbox" checked={folder.orbitIds.includes(orbit.id)}
              onChange={event => setFolder(value => ({ ...value, orbitIds: event.target.checked ? [...value.orbitIds, orbit.id] : value.orbitIds.filter(id => id !== orbit.id) }))} /><span>{orbit.name}</span></label>) : <p>Create a Sprite first, or choose all Sprites.</p>}
          </div>}
        </fieldset>
        <label className="shared-folder-write"><input type="checkbox" disabled={disabled} checked={folder.writable}
          onChange={event => setFolder(value => ({ ...value, writable: event.target.checked }))} /><span><strong>Allow edits to the originals</strong><small>{folder.writable ? "Changes and deletions by these Sprites also affect the original files on your computer." : "Read-only. Sprites can use these files without changing the originals."}</small></span></label>
        <details className="shared-folder-advanced"><summary>Workspace location</summary><label className="field"><span>Link inside each workspace</span><input value={folder.mountPath} disabled={disabled}
          placeholder={suggestedMount} onChange={event => setFolder(value => ({ ...value, mountPath: event.target.value }))} /><small>Relative to the Sprite’s workspace. Existing files are never silently replaced.</small></label></details>
        <p className="shared-folder-notice"><Link2 size={15} /><span>This is a symlink, not a copy. Updates to the original appear in every linked workspace. Share configuration and credential folders only with Sprites you trust.</span></p>
      </div>
      {error && <p className="form-error" role="alert">{error}</p>}
      <footer className="form-footer"><button type="button" className="button button-secondary" disabled={disabled} onClick={close}>Cancel</button>
        <button type="submit" className="button button-primary" disabled={disabled || (!folder.allSprites && !folder.orbitIds.length)}>{busy && <LoaderCircle size={14} className="spin" />}{existing ? "Save sharing" : "Create workspace links"}</button></footer>
    </form>
  </Modal>;
}

export interface HomeSharingPlan { count: number; commit: () => Promise<boolean>; }

export function HomeSharingSetup({ data, refresh, onBusyChange, onPlanChange }: {
  data: Bootstrap; refresh: () => Promise<void>; onBusyChange?: (busy: boolean) => void;
  onPlanChange?: (plan: HomeSharingPlan | null) => void;
}) {
  const [candidates, setCandidates] = useState<HomeShareCandidate[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [mountPaths, setMountPaths] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const shares = data.sharedFolders ?? [];
  const alreadyShared = (candidate: HomeShareCandidate) => shares.some(folder => folder.allSprites && samePath(folder.directory, candidate.directory));
  const load = useCallback(async () => {
    setLoading(true); setError("");
    try { setCandidates(await api<HomeShareCandidate[]>("discover_home_shares")); }
    catch (reason) { setError(errorText(reason)); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => { onBusyChange?.(saving); return () => onBusyChange?.(false); }, [saving, onBusyChange]);
  const chosen = candidates.filter(candidate => selected.includes(candidate.id) && !alreadyShared(candidate));
  const commit = useRef<() => Promise<boolean>>(async () => false);
  useEffect(() => { onPlanChange?.({ count: chosen.length, commit: () => commit.current() }); }, [chosen.length, onPlanChange]);
  useEffect(() => () => onPlanChange?.(null), [onPlanChange]);
  async function shareSelected() {
    if (!chosen.length || saving) return !saving;
    setSaving(true); setError(""); setSaved(false);
    try {
      for (const candidate of chosen) {
        await api("save_shared_folder", { folder: {
          id: crypto.randomUUID(), name: candidate.name, directory: candidate.directory, mountPath: mountPaths[candidate.id]?.trim() || candidate.mountPath,
          allSprites: true, orbitIds: [], writable: false,
        } satisfies SharedFolder });
      }
      setSelected([]); setSaved(true);
      return true;
    } catch (reason) { setError(errorText(reason)); return false; }
    finally { await refresh(); setSaving(false); }
  }
  commit.current = shareSelected;
  return <section className="home-sharing-setup" aria-labelledby="home-sharing-heading">
    <div className="shared-folder-heading"><div><h2 id="home-sharing-heading"><Link2 size={18} />Familiar tools, in every workspace</h2><p>Choose which configuration files and folders to link for all current and future Sprites.</p></div></div>
    <p className="shared-folder-discovery-note">Solar checks common locations by name, without opening their files. Nothing is shared until you choose it.</p>
    {loading ? <p className="shared-folder-status" role="status"><LoaderCircle size={16} className="spin" />Looking for familiar configuration folders…</p> : candidates.length ? <div className="home-sharing-candidates">
      {candidates.map(candidate => { const linked = alreadyShared(candidate); return <label key={candidate.id} className={linked ? "already-shared" : ""}>
        <input type="checkbox" disabled={saving || linked} checked={linked || selected.includes(candidate.id)} onChange={event => { setSaved(false); setSelected(value => event.target.checked ? [...value, candidate.id] : value.filter(id => id !== candidate.id)); }} />
        <span><strong>{candidate.name}{linked && <em><Check size={12} />Shared</em>}</strong><code>{candidate.directory}</code><small>{candidate.description}</small>{candidate.sensitive && <small className="shared-folder-sensitive"><LockKeyhole size={12} />May contain credentials or private instructions.</small>}</span>
      </label>; })}
    </div> : !error && <p className="shared-folder-status">{native ? "No common configuration folders found. You can add any folder later in Projects." : "Open the desktop app to discover folders on your computer. You can still preview sharing in Projects."}</p>}
    {chosen.length > 0 && <details className="shared-folder-advanced home-sharing-paths" open={Boolean(error)}>
      <summary>Customize workspace links</summary>
      <p>If a Sprite already has its own configuration at a location, choose an unused path such as <code>shared/kube</code>. Existing private files are kept.</p>
      {chosen.map(candidate => <label className="field" key={candidate.id}><span>{candidate.name}</span><input disabled={saving} value={mountPaths[candidate.id] ?? candidate.mountPath}
        aria-label={`Workspace link for ${candidate.name}`} onChange={event => setMountPaths(value => ({ ...value, [candidate.id]: event.target.value }))} /></label>)}
    </details>}
    <p className="shared-folder-notice"><LockKeyhole size={15} /><span>These links are read-only by default. Manage access, enable edits, or remove a link later in <strong>Projects → Shared folders</strong>.</span></p>
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="shared-folder-setup-actions">
      <button type="button" className="button button-primary" disabled={loading || saving || !chosen.length} onClick={() => void shareSelected()}>{saving ? <LoaderCircle size={15} className="spin" /> : <Link2 size={15} />}{saving ? "Creating workspace links…" : chosen.length ? `Share ${chosen.length} selected ${chosen.length === 1 ? "item" : "items"}` : "Choose items to share"}</button>
      {error && <button type="button" className="button button-quiet" disabled={saving || loading} onClick={() => void load()}><RefreshCw size={14} />Try again</button>}
      {saved && <span role="status"><Check size={14} />Workspace links are ready.</span>}
    </div>
  </section>;
}

export function SharedFoldersPanel({ data, refresh }: { data: Bootstrap; refresh: () => Promise<void> }) {
  const [editing, setEditing] = useState<SharedFolder | "new" | null>(null);
  const [removing, setRemoving] = useState<SharedFolder | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [discovering, setDiscovering] = useState(false);
  const shares = data.sharedFolders ?? [];
  async function remove() {
    if (!removing || busy) return;
    setBusy(true); setError("");
    try { await api("delete_shared_folder", { id: removing.id }); await refresh(); setRemoving(null); }
    catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }
  return <section className="shared-folders-panel" aria-labelledby="shared-folders-heading">
    <div className="shared-folder-heading"><div><h2 id="shared-folders-heading">Shared folders</h2><p>Live links to files on your computer, available inside selected Sprites’ workspaces.</p></div>
      <button type="button" className="button button-secondary" onClick={() => setEditing("new")}><Plus size={15} />Share a folder</button></div>
    {shares.length ? <div className="shared-folders-grid">{shares.map(folder => <article className="shared-folder-card" key={folder.id}>
      <div className="shared-folder-card-top"><Link2 size={19} /><span className={`shared-folder-access ${folder.writable ? "writable" : ""}`}>{folder.writable ? "Can edit originals" : "Read-only"}</span></div>
      <h3>{folder.name}</h3><code>{folder.directory}</code><p className="shared-folder-location">Workspace link <code>~/{folder.mountPath}</code></p>
      <p className="shared-folder-people"><Users size={14} />{folder.allSprites ? "All current and future Sprites" : folder.orbitIds.map(id => data.orbits.find(orbit => orbit.id === id)?.name).filter(Boolean).join(", ") || "No Sprites selected"}</p>
      <div className="button-row"><button type="button" className="button button-secondary button-small" onClick={() => setEditing(folder)}><Settings2 size={14} />Edit sharing</button><button type="button" className="icon-button danger-text" aria-label={`Stop sharing ${folder.name}`} onClick={() => { setError(""); setRemoving(folder); }}><Trash2 size={15} /></button></div>
    </article>)}</div> : <div className="shared-folder-empty"><FolderOpen size={25} /><div><strong>Bring useful files into your Sprites’ workspaces.</strong><p>Choose a folder once, then decide who can use it. Your original files stay in place.</p></div></div>}
    <button type="button" className="button button-quiet shared-folder-discover-toggle" aria-expanded={discovering} onClick={() => setDiscovering(value => !value)}><FolderOpen size={14} />{discovering ? "Hide common configuration folders" : "Find common configuration folders"}</button>
    {discovering && <HomeSharingSetup data={data} refresh={refresh} />}
    {editing && <SharedFolderForm existing={editing === "new" ? undefined : editing} data={data} onClose={() => setEditing(null)} onSaved={refresh} />}
    {removing && <Modal title={`Stop sharing ${removing.name}?`} subtitle="This removes workspace links. The original files on your computer are kept." onClose={() => { if (!busy) setRemoving(null); }} dismissible={!busy}>
      <div className="form-body"><p>Sprites will no longer have access through this shared folder. Existing task results and any separate copies are kept.</p>{error && <p className="form-error" role="alert">{error}</p>}</div>
      <footer className="form-footer"><button type="button" className="button button-secondary" disabled={busy} onClick={() => setRemoving(null)}>Cancel</button><button type="button" className="button button-primary" disabled={busy} onClick={() => void remove()}>{busy && <LoaderCircle size={14} className="spin" />}Stop sharing</button></footer>
    </Modal>}
  </section>;
}
