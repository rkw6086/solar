import { useCallback, useEffect, useRef, useState } from "react";
import { api, native } from "../api";
import { errorText } from "../types";

// Serialize the native changes across remounts. In development, StrictMode can
// mount, clean up, and mount again before the first IPC request has finished.
const leases = new Map<symbol, boolean>();
let changes = Promise.resolve();

function reconcile(): Promise<void> {
  const operation = changes.then(async () => {
    const active = [...leases.values()].some(Boolean);
    // Native close/hide handling can restore the window independently. Send
    // the desired lease state each time; the native controller is idempotent.
    await api("set_intro_fullscreen", { active });
  });
  // A failed request must not prevent a later restore/retry from running.
  changes = operation.catch(() => {});
  return operation;
}

function release(lease: symbol): Promise<void> {
  leases.delete(lease);
  return reconcile();
}

/** Lease native fullscreen for the entire welcome tour, including setup forms. */
export function useIntroFullscreen() {
  const lease = useRef(Symbol("welcome-fullscreen"));
  const released = useRef(false);
  const [ready, setReady] = useState(!native);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    released.current = false;
    const update = () => {
      if (released.current) return;
      leases.set(lease.current, document.visibilityState === "visible");
      void reconcile().then(() => {
        if (!disposed) {
          setReady(true);
          setError("");
        }
      }).catch((reason) => {
        // Fullscreen failure should not stop setup in the existing window.
        if (!disposed) {
          setReady(true);
          setError(errorText(reason));
        }
      });
    };
    const hide = () => {
      leases.set(lease.current, false);
      void reconcile().catch(() => {});
    };
    document.addEventListener("visibilitychange", update);
    window.addEventListener("pagehide", hide);
    update();
    return () => {
      disposed = true;
      released.current = true;
      document.removeEventListener("visibilitychange", update);
      window.removeEventListener("pagehide", hide);
      void release(lease.current).catch(() => {});
    };
  }, []);

  const leaveFullscreen = useCallback(async () => {
    released.current = true;
    if (native) await release(lease.current);
  }, []);

  return { ready, error, leaveFullscreen };
}
