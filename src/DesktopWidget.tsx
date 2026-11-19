import { useEffect, useRef, useState } from "react";
import type { CSSProperties, PointerEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowUpRight, LoaderCircle, Sun } from "lucide-react";
import { api, native } from "./api";
import { useSolar } from "./useSolar";
import { activeTask, errorText } from "./types";
import type { Conversation } from "./types";
import { Planet } from "./components/Planet";
import { Chat } from "./components/Chat";

function useCharacterDrag() {
  const start = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);
  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    start.current = { x: event.screenX, y: event.screenY };
    dragged.current = false;
  }
  function onPointerMove(event: PointerEvent) {
    if ((event.buttons & 1) === 0) {
      start.current = null;
      return;
    }
    if (!start.current || dragged.current || !native) return;
    if (
      Math.hypot(
        event.screenX - start.current.x,
        event.screenY - start.current.y,
      ) > 5
    ) {
      dragged.current = true;
      start.current = null;
      void getCurrentWindow()
        .startDragging()
        .catch(() => {});
    }
  }
  function wasClick() {
    start.current = null;
    return !dragged.current;
  }
  return { onPointerDown, onPointerMove, wasClick };
}

export function DesktopWidget({
  type,
  orbitId,
}: {
  type: "sun" | "orbit" | "chat";
  orbitId?: string;
}) {
  const { data, loading, error } = useSolar();
  const [expanded, setExpanded] = useState(false);
  const [failure, setFailure] = useState("");
  const [sunMood, setSunMood] = useState<"smile" | "yawn" | "neutral">("neutral");
  const [sunAntic, setSunAntic] = useState("");
  const [popupPhase, setPopupPhase] = useState("opening");
  const [popupOrigin, setPopupOrigin] = useState({ x: 206, y: 216 });
  const lastInteraction = useRef(0);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const openingConversation = useRef<{
    orbitId: string;
    promise: Promise<Conversation>;
  } | null>(null);
  const drag = useCharacterDrag();
  const orbit = data.orbits.find((item) => item.id === orbitId);
  const visibleCount = data.orbits.filter(item => item.enabled).length;
  const planetSize = visibleCount > 36 ? 56 : visibleCount > 18 ? 72 : 88;

  function interaction(hovered?: boolean) {
    if (!native) return;
    const now = Date.now();
    if (hovered === undefined && now - lastInteraction.current < 1000) return;
    lastInteraction.current = now;
    void api("desktop_interaction", hovered === undefined ? {} : { hovered }).catch(() => {});
  }

  useEffect(() => {
    if (type !== "sun" || expanded || data.settings.reducedMotion) return;
    let reset: ReturnType<typeof setTimeout>;
    const timer = setInterval(() => {
      const choice = Math.floor(Math.random() * 3);
      setSunMood(choice === 1 ? "yawn" : "smile");
      setSunAntic(choice === 0 ? "sun-roll" : choice === 1 ? "sun-yawn" : "sun-rocket");
      reset = setTimeout(() => { setSunMood("neutral"); setSunAntic(""); }, 3200);
    }, 18000);
    return () => { clearInterval(timer); clearTimeout(reset); setSunAntic(""); setSunMood("neutral"); };
  }, [type, expanded, data.settings.reducedMotion]);

  useEffect(() => {
    if (type !== "chat" || loading) return;
    if (!native) { setPopupPhase("open"); return; }
    let disposed = false;
    let cleanup: (() => void) | undefined;
    let frame = 0;
    void Promise.all([
      listen<{ x: number; y: number }>("solar://popup-close", ({ payload }) => {
        setPopupOrigin(payload);
        setPopupPhase("closing");
      }),
      listen("solar://popup-open", () => setPopupPhase("open")),
    ]).then(async (unlisteners) => {
      const unlisten = () => unlisteners.forEach(stop => stop());
      if (disposed) { unlisten(); return; }
      cleanup = unlisten;
      const origin = await api<{ x: number; y: number }>("popup_ready");
      if (disposed) return;
      setPopupOrigin(origin);
      frame = requestAnimationFrame(() => { frame = requestAnimationFrame(() => setPopupPhase("open")); });
    }).catch(reason => { if (!disposed) setFailure(errorText(reason)); });
    return () => { disposed = true; cleanup?.(); cancelAnimationFrame(frame); };
  }, [type, loading]);

  useEffect(() => {
    document.documentElement.dataset.motion = data.settings.reducedMotion ? "reduced" : "full";
  }, [data.settings.reducedMotion]);

  useEffect(() => {
    if (!native || loading || error || type === "chat" || (type === "orbit" && !orbit)) return;
    void api("widget_ready").catch((reason) => setFailure(errorText(reason)));
  }, [loading, error, type, orbit?.id]);

  useEffect(() => {
    if (typeof data.desktopExpanded === "boolean")
      setExpanded(data.desktopExpanded);
  }, [data.desktopExpanded]);

  useEffect(() => {
    if (type !== "chat" || !orbitId || loading) return;
    let current = true;
    const existing = data.conversations.find(
      (item) => item.orbitIds.length === 1 && item.orbitIds[0] === orbitId,
    );
    if (existing) {
      setConversation(existing);
      return;
    }
    if (openingConversation.current?.orbitId !== orbitId) {
      openingConversation.current = {
        orbitId,
        promise: api<Conversation>("create_conversation", {
          orbitIds: [orbitId],
          leadOrbitId: orbitId,
          title: orbit?.name ?? "Sprite",
          projectId: undefined,
        }),
      };
    }
    void openingConversation.current.promise
      .then((result) => {
        if (current) setConversation(result);
      })
      .catch((reason) => {
        if (current) setFailure(errorText(reason));
      });
    return () => {
      current = false;
    };
  }, [type, orbitId, loading, orbit?.name, data.conversations]);

  async function toggle(keyboard = false) {
    if (!keyboard && !drag.wasClick()) return;
    try {
      await api("set_desktop_expanded", { expanded: !expanded });
      setExpanded(!expanded);
    } catch (reason) {
      setFailure(errorText(reason));
    }
  }

  if (type === "chat")
    return (
      <div className={`floating-chat-shell popup-${popupPhase}`} style={{ "--popup-x": `${popupOrigin.x}px`, "--popup-y": `${popupOrigin.y}px` } as CSSProperties}
        onPointerDown={() => interaction()} onKeyDown={() => interaction()}>
      <div className="popup-planet" aria-hidden="true"><Planet color={orbit?.color} name={orbit?.name} identity={orbit?.id} size={planetSize} /></div>
      <div className="floating-chat">
        {conversation ? (
          <Chat
            conversation={conversation}
            data={data}
            compact
            onOrbitChanged={() => { openingConversation.current = null; setConversation(null); }}
            onExpand={() =>
              void api("open_full_chat", {
                conversationId: conversation.id,
              }).catch((reason) => setFailure(errorText(reason)))
            }
          />
        ) : (
          <div className="widget-loading">
            <LoaderCircle className="spin" size={20} />
            <span>{failure || error || "Opening conversation…"}</span>
          </div>
        )}
        {failure && conversation && <p className="widget-error">{failure}</p>}
      </div>
      </div>
    );

  if (type === "sun")
    return (
      <div className={`sun-widget ${sunAntic}`} onPointerEnter={() => interaction(true)} onPointerLeave={() => interaction(false)} onPointerMove={() => interaction()}>
        <div className="pixel-rocket-track" aria-hidden="true"><svg className="pixel-rocket" viewBox="0 0 16 20" shapeRendering="crispEdges"><path fill="#d4dfdf" d="M6 1h4v2h2v9H4V3h2z"/><path fill="#7cc3d2" d="M6 5h4v4H6z"/><path fill="#d68777" d="M2 10h3v6H1v-3h1zm9 0h3v3h1v3h-4z"/><path fill="#efbb69" d="M5 13h6v3H5zm2 3h2v3H7z"/></svg></div>
        <div className="widget-orbit-path widget-orbit-one" />
        <div className="widget-orbit-path widget-orbit-two" />
        <button
          className="character-button sun-character"
          aria-label={expanded ? "Gather Sprites" : "Expand Sprites"}
          onPointerDown={drag.onPointerDown}
          onPointerMove={drag.onPointerMove}
          onClick={(event) => void toggle(event.detail === 0)}
          onContextMenu={(event) => {
            event.preventDefault();
            void api("open_launcher");
          }}
        >
          <Planet sun color="#e8aa62" size={100} name="Solar sun" mood={sunMood} />
          <span className="sun-widget-label">
            {expanded ? "Gather Sprites" : "Your Sprites"}
          </span>
        </button>
        <button
          className="widget-launcher"
          title="Open Solar"
          aria-label="Open Solar launcher"
          onClick={() => void api("open_launcher")}
        >
          <ArrowUpRight size={15} />
        </button>
        {failure && <span className="widget-error">{failure}</span>}
      </div>
    );

  if (!orbit)
    return (
      <button
        className="character-button"
        onClick={() => void api("open_launcher")}
        aria-label="Open Solar"
      >
        <Sun size={40} />
      </button>
    );
  const busy = data.tasks.some(
    (task) => task.orbitId === orbit.id && activeTask(task),
  );
  const needsAttention =
    Boolean(data.attentionItems?.some((item) => item.orbitId === orbit.id)) ||
    data.approvals.some((approval) => approval.orbitId === orbit.id);
  return (
    <div className={`orbit-widget ${needsAttention ? "needs-attention" : ""}`} style={{ "--orbit-planet-size": `${planetSize}px` } as CSSProperties} onPointerEnter={() => interaction(true)} onPointerLeave={() => interaction(false)} onPointerMove={() => interaction()}>
      <button
        className="character-button"
        aria-label={`Chat with ${orbit.name}${needsAttention ? ", needs your attention" : ""}`}
        onClick={() => {
          void api("open_orbit_chat", { orbitId: orbit.id }).catch((reason) =>
            setFailure(errorText(reason)),
          );
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          void api("open_orbit_settings", { orbitId: orbit.id }).catch((reason) => setFailure(errorText(reason)));
        }}
        title={`${orbit.name} · right-click for settings`}
      >
        <Planet color={orbit.color} size={planetSize} name={orbit.name} identity={orbit.id} busy={busy} />
        <span className="orbit-widget-name">
          {orbit.name}
          {needsAttention ? (
            <span className="orbit-attention-pip" />
          ) : busy && <span className="working-pip" />}
        </span>
      </button>
      {failure && <span className="widget-error">{failure}</span>}
    </div>
  );
}
