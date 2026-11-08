import { useEffect, useRef, useState } from "react";
import type { PointerEvent } from "react";
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
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const openingConversation = useRef<{
    orbitId: string;
    promise: Promise<Conversation>;
  } | null>(null);
  const drag = useCharacterDrag();
  const orbit = data.orbits.find((item) => item.id === orbitId);

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
          title: orbit?.name ?? "Orbit",
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
      <div className="floating-chat">
        {conversation ? (
          <Chat
            conversation={conversation}
            data={data}
            compact
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
    );

  if (type === "sun")
    return (
      <div className="sun-widget">
        <div className="widget-orbit-path widget-orbit-one" />
        <div className="widget-orbit-path widget-orbit-two" />
        <button
          className="character-button sun-character"
          aria-label={expanded ? "Gather Orbits" : "Expand Orbits"}
          onPointerDown={drag.onPointerDown}
          onPointerMove={drag.onPointerMove}
          onClick={(event) => void toggle(event.detail === 0)}
          onContextMenu={(event) => {
            event.preventDefault();
            void api("open_launcher");
          }}
        >
          <Planet sun color="#e8aa62" size={100} name="Solar sun" />
          <span className="sun-widget-label">
            {expanded ? "Gather Orbits" : "Your Orbits"}
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
    <div className={`orbit-widget ${needsAttention ? "needs-attention" : ""}`}>
      <button
        className="character-button"
        aria-label={`Chat with ${orbit.name}${needsAttention ? ", needs your attention" : ""}`}
        onClick={() => {
          void api("open_orbit_chat", { orbitId: orbit.id }).catch((reason) =>
            setFailure(errorText(reason)),
          );
        }}
      >
        <Planet color={orbit.color} size={88} name={orbit.name} busy={busy} />
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
