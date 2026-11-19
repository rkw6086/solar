import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowUpRight, Bell, X } from "lucide-react";
import { api, native } from "./api";
import { useSolar } from "./useSolar";
import { errorText } from "./types";
import type { AttentionItem, AttentionState } from "./types";
import { Planet } from "./components/Planet";
import "./attention.css";

const kindLabels: Record<AttentionItem["kind"], string> = {
  approval: "Permission needed",
  failed: "Needs a hand",
  message: "Something to share",
};

export function AttentionWidget() {
  const { data, loading } = useSolar();
  const [state, setState] = useState<AttentionState | null>(null);
  const [displayed, setDisplayed] = useState<AttentionItem | null>(null);
  const [leaving, setLeaving] = useState(false);
  const [failure, setFailure] = useState("");
  const [pending, setPending] = useState(false);
  const transition = useRef<ReturnType<typeof setTimeout> | null>(null);
  const currentId = useRef<string | undefined>(undefined);
  const capsule = useRef<HTMLElement | null>(null);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    const update = (value: AttentionState) => {
      if (!disposed) {
        setState(value);
        setFailure("");
      }
    };
    async function connect() {
      if (native) {
        const unlisten = await listen<AttentionState>("solar://attention", (event) =>
          update(event.payload),
        );
        if (disposed) unlisten();
        else cleanup = unlisten;
      } else {
        const handle = (event: Event) =>
          update((event as CustomEvent<AttentionState>).detail);
        window.addEventListener("solar-preview-attention", handle);
        cleanup = () =>
          window.removeEventListener("solar-preview-attention", handle);
      }
      update(await api<AttentionState>("get_attention_state"));
    }
    void connect().catch((reason) => {
      if (!disposed) setFailure(errorText(reason));
    });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  const incoming = state?.current ?? state?.items[0];
  useEffect(() => {
    if (transition.current) clearTimeout(transition.current);
    if (incoming) {
      currentId.current = incoming.id;
      setDisplayed(incoming);
      setLeaving(false);
      setPending(false);
      setFailure("");
    } else if (currentId.current) {
      currentId.current = undefined;
      setLeaving(true);
      transition.current = setTimeout(
        () => setDisplayed(null),
        state?.reducedMotion ? 0 : 180,
      );
    }
    return () => {
      if (transition.current) clearTimeout(transition.current);
    };
  }, [incoming?.id, state?.reducedMotion]);

  useEffect(() => {
    if (!state || loading || (incoming && displayed?.id !== incoming.id)) return;
    document.documentElement.dataset.motion = state.reducedMotion
      ? "reduced"
      : "full";
    if (native) {
      void api("attention_ready").catch((reason) => setFailure(errorText(reason)));
    }
  }, [Boolean(state), loading, state?.reducedMotion, incoming?.id, displayed?.id]);

  useEffect(() => {
    const element = capsule.current;
    if (!native || !element) return;
    let previousHeight = 0;
    const measure = () => {
      // offsetHeight remains stable while the entrance transform scales the card.
      const height = Math.ceil(
        Math.max(element.offsetHeight, element.getBoundingClientRect().height),
      );
      if (height === previousHeight) return;
      previousHeight = height;
      void api("attention_bounds", { height }).catch(() => {});
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    const frame = requestAnimationFrame(measure);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [displayed?.id]);

  function act(command: "dismiss_attention" | "open_attention") {
    if (!displayed || pending) return;
    const id = displayed.id;
    setPending(true);
    setLeaving(true);
    transition.current = setTimeout(() => {
      void api(command, { id }).catch((reason) => {
        if (currentId.current === id) {
          setFailure(errorText(reason));
          setLeaving(false);
          setPending(false);
        }
      });
    }, state?.reducedMotion ? 0 : 180);
  }

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (event.key === "Escape" && displayed && !pending) {
        event.preventDefault();
        act("dismiss_attention");
      }
    };
    window.addEventListener("keydown", handle);
    return () => window.removeEventListener("keydown", handle);
  });

  if (!state || !displayed) {
    return failure ? (
      <div className="attention-load-error" role="status">{failure}</div>
    ) : null;
  }
  const orbit = data.orbits.find((item) => item.id === displayed.orbitId);
  const name = orbit?.name ?? "Your Sprite";
  const color = orbit?.color ?? "#e8aa62";
  const remaining = Math.max(0, state.items.length - 1);
  const style = {
    "--attention-color": color,
    "--notch-height": `${state.geometry.notchHeight}px`,
    "--notch-width": `${state.geometry.notchWidth}px`,
  } as CSSProperties;

  return (
    <div className="attention-stage" style={style}>
      <section
        ref={capsule}
        key={displayed.id}
        className={`attention-capsule ${state.geometry.mode === "notch" ? "has-notch" : "at-edge"} ${leaving ? "is-leaving" : ""} ${state.reducedMotion ? "reduce-motion" : ""}`}
        aria-label={`${name} needs your attention`}
        aria-live="polite"
      >
        <div className="attention-hardware-space" aria-hidden="true" />
        <div className="attention-content">
          <button
            type="button"
            className="attention-dismiss"
            onClick={() => act("dismiss_attention")}
            aria-label={`Dismiss ${name}'s notification`}
            title="Dismiss notification (Escape)"
            disabled={pending}
          >
            <X size={14} />
          </button>
          <button
            type="button"
            className="attention-open"
            onClick={() => act("open_attention")}
            aria-label={`Open ${name}'s conversation: ${displayed.title}`}
            disabled={pending}
          >
            <span className="attention-planet">
              <Planet identity={orbit?.id} color={color} name={name} size={68} />
              <span className="attention-spark attention-spark-one" aria-hidden="true" />
              <span className="attention-spark attention-spark-two" aria-hidden="true" />
            </span>
            <span className="attention-eyebrow">
              {name} <span>·</span> {kindLabels[displayed.kind]}
            </span>
            <span className="attention-title">{displayed.title}</span>
            <span className="attention-action">
              Open conversation <ArrowUpRight size={13} />
            </span>
          </button>
          {remaining > 0 && (
            <div className="attention-queue">
              <Bell size={11} />{remaining} more {remaining === 1 ? "update" : "updates"} waiting
            </div>
          )}
          {failure && (
            <div className="attention-failure" role="alert">{failure}</div>
          )}
        </div>
      </section>
    </div>
  );
}
