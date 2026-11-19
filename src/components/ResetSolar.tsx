import { useState } from "react";
import { LoaderCircle, RotateCcw } from "lucide-react";
import { Modal } from "./Modal";
import { errorText } from "../types";

export function ResetSolar({ onReset }: { onReset: () => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const close = () => { if (!busy) setOpen(false); };

  async function reset() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await onReset();
      // The native app restarts. In browser preview the parent remounts setup.
      setOpen(false);
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <section className="settings-section">
        <h2>Start over</h2>
        <div className="settings-row">
          <div>
            <h3>Reset Solar</h3>
            <p>Erase Solar’s local data and connections, then restart with the opening animation and setup.</p>
          </div>
          <button className="button button-secondary danger-text" onClick={() => { setError(""); setOpen(true); }} type="button">
            <RotateCcw size={14} /> Reset Solar
          </button>
        </div>
      </section>
      {open && (
        <Modal title="Reset Solar?" subtitle="Solar will restart from the beginning." onClose={close}>
          <div className="form-body">
            <p className="confirm-description">
              This permanently removes your Sprites, conversations, memories, routines, settings, saved sign-ins, and files inside Solar’s private workspaces. Files in external project folders stay where they are.
            </p>
            {error && <p className="form-error" role="alert">{error}</p>}
            {busy && <p className="muted" role="status">Stopping active work and preparing to restart…</p>}
            <div className="button-row justify-end">
              <button className="button button-secondary" type="button" onClick={close} disabled={busy}>Cancel</button>
              <button className="button button-primary" type="button" onClick={() => void reset()} disabled={busy} style={{ background: "#ad5143", color: "#fff5ef" }}>
                {busy ? <LoaderCircle size={14} className="spin" /> : <RotateCcw size={14} />}
                {busy ? "Resetting…" : "Reset Solar"}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
