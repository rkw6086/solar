import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  ArrowUp,
  Check,
  Expand,
  LoaderCircle,
  Monitor,
  Settings2,
  ShieldCheck,
  Square,
  Users,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, native } from "../api";
import { activeTask, errorText, isProviderConfigured } from "../types";
import type { Bootstrap, Conversation, Message } from "../types";
import { Planet } from "./Planet";
import { mergeChatMessages } from "./chatMessages";
import { OrbitSettingsPanel } from "./OrbitSettingsPanel";
import { ToolActivityPanel, ToolApprovalDetails } from "./ToolActivityPanel";

let presenceRevision = Date.now() * 1_000;
const nextPresenceRevision = () => (presenceRevision = Math.max(presenceRevision + 1, Date.now() * 1_000));

export function Chat({
  conversation,
  data,
  onBack,
  compact = false,
  onExpand,
  onConnections,
  onOrbitChanged,
}: {
  conversation: Conversation;
  data: Bootstrap;
  onBack?: () => void;
  compact?: boolean;
  onExpand?: () => void;
  onConnections?: () => void;
  onOrbitChanged?: () => void;
}) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sharingBusy, setSharingBusy] = useState(false);
  const [error, setError] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);
  const messagePane = useRef<HTMLDivElement>(null);
  const messageContent = useRef<HTMLDivElement>(null);
  const userScrollUntil = useRef(0);
  const touchY = useRef(0);
  const headerDrag = useRef<{ x: number; y: number } | null>(null);
  const followMessages = useRef(true);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const currentRequest = useRef(0);
  const liveMessages = useRef(new Map<string, Message>());
  const orbit = data.orbits.find(
    (item) => item.id === conversation.leadOrbitId,
  );
  const team = data.orbits.filter((item) =>
    conversation.orbitIds.includes(item.id),
  );
  const running = data.tasks.filter(
    (task) => task.conversationId === conversation.id && activeTask(task),
  );
  const approvals = data.approvals.filter((approval) =>
    running.some((task) => task.id === approval.taskId),
  );
  const sharing = (data.sharingOrbitIds ?? []).includes(
    conversation.leadOrbitId,
  );
  const connected = data.providers.some(
    (provider) =>
      provider.id === orbit?.providerId && isProviderConfigured(provider),
  );

  useEffect(() => {
    if (!native) return;
    void api("set_active_chat", {
      conversationId: conversation.id,
      revision: nextPresenceRevision(),
    }).catch(() => {});
    return () => {
      void api("set_active_chat", {
        conversationId: null,
        revision: nextPresenceRevision(),
      }).catch(() => {});
    };
  }, [conversation.id]);

  useEffect(() => {
    const sequence = ++currentRequest.current;
    void api<Message[]>("get_messages", { conversationId: conversation.id })
      .then((result) => {
        if (sequence === currentRequest.current) {
          setMessages(mergeChatMessages(result, liveMessages.current.values(), conversation.id));
        }
      })
      .catch((failure) => {
        if (sequence === currentRequest.current) setError(errorText(failure));
      });
  }, [conversation.id, data]);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void listen<Message>("solar://message", ({ payload }) => {
      if (disposed || payload.conversationId !== conversation.id) return;
      liveMessages.current.set(payload.id, payload);
      setMessages(current => current.some(message => message.id === payload.id)
        ? current.map(message => message.id === payload.id ? payload : message)
        : [...current, payload]);
    }).then(unlisten => { if (disposed) unlisten(); else cleanup = unlisten; }).catch(() => {});
    return () => { disposed = true; cleanup?.(); };
  }, [conversation.id]);

  function scrollToLatest() {
    const pane = messagePane.current;
    if (pane && followMessages.current) pane.scrollTop = pane.scrollHeight;
  }
  useLayoutEffect(() => { scrollToLatest(); }, [messages, data.tasks, data.approvals]);
  useLayoutEffect(() => {
    followMessages.current = true;
    liveMessages.current.clear();
    userScrollUntil.current = 0;
    setDraft("");
    setError("");
    scrollToLatest();
  }, [conversation.id]);
  useEffect(() => {
    const content = messageContent.current;
    if (!content) return;
    // Markdown, images, expanding tools and streamed text can change height
    // without changing the outer message list's dimensions or identity.
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(scrollToLatest);
    });
    observer.observe(content);
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [conversation.id]);

  async function send() {
    if (!draft.trim() || sending) return;
    const content = draft.trim();
    setSending(true);
    setError("");
    try {
      await api("send_message", { conversationId: conversation.id, content });
      setDraft("");
      const result = await api<Message[]>("get_messages", { conversationId: conversation.id });
      setMessages(mergeChatMessages(result, liveMessages.current.values(), conversation.id));
      textarea.current?.focus();
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setSending(false);
    }
  }

  async function action(command: string, args: Record<string, unknown>) {
    try {
      setError("");
      await api(command, args);
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  async function toggleSharing() {
    if (sharingBusy) return;
    setSharingBusy(true);
    try {
      await action(sharing ? "stop_screen_share" : "start_screen_share", {
        orbitId: conversation.leadOrbitId,
      });
    } finally {
      setSharingBusy(false);
    }
  }

  return (
    <section className={`chat-view ${compact ? "chat-compact" : ""}`}>
      <header className={`chat-header ${compact ? "chat-drag-handle" : ""}`}
        onPointerDown={(event) => {
          if (!compact || !native || event.button !== 0 || (event.target as HTMLElement).closest("button,a,input,textarea,select")) return;
          headerDrag.current = { x: event.screenX, y: event.screenY };
        }}
        onPointerMove={(event) => {
          const start = headerDrag.current;
          if (!(event.buttons & 1)) { headerDrag.current = null; return; }
          if (start && Math.hypot(event.screenX - start.x, event.screenY - start.y) > 5) {
            headerDrag.current = null;
            void getCurrentWindow().startDragging().catch(() => {});
          }
        }}
        onPointerUp={() => { headerDrag.current = null; }}
        title={compact ? "Drag here to move this conversation" : undefined}>
        {onBack && (
          <button
            className="icon-button"
            onClick={onBack}
            aria-label="Back to conversations"
          >
            <ArrowLeft size={18} />
          </button>
        )}
        <div className="chat-team-avatar">
          {team.slice(0, 3).map((item) => (
            <Planet
              key={item.id}
              name={item.name}
              identity={item.id}
              color={item.color}
              size={42}
            />
          ))}
        </div>
        <div className="chat-heading">
          <h2>{conversation.title}</h2>
          <p>
            {team.length > 1
              ? `${team.map((item) => item.name).join(", ")} · ${orbit?.name ?? "Sprite"} leads`
              : (orbit?.role ?? "Sprite conversation")}
          </p>
        </div>
        {orbit && <button
          type="button"
          className="icon-button"
          onClick={() => setSettingsOpen(true)}
          aria-label={`Settings for ${orbit.name}`}
          title={`Settings for ${orbit.name}`}
        ><Settings2 size={17} /></button>}
        {onExpand && (
          <button
            className="icon-button"
            onClick={onExpand}
            aria-label="Expand conversation"
          >
            <Expand size={16} />
          </button>
        )}
        <span
          className={`connection-dot ${connected ? "online" : ""}`}
          title={connected ? "Model connected" : "Model connection needed"}
        />
      </header>
      <div
        className="chat-messages"
        ref={messagePane}
        tabIndex={0}
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
        onWheel={(event) => {
          userScrollUntil.current = Date.now() + 500;
          if (event.deltaY < 0) followMessages.current = false;
        }}
        onKeyDown={(event) => {
          if (["ArrowUp", "PageUp", "Home"].includes(event.key)) {
            userScrollUntil.current = Date.now() + 500;
            followMessages.current = false;
          }
          if (["ArrowDown", "PageDown", "End"].includes(event.key)) userScrollUntil.current = Date.now() + 500;
        }}
        onTouchStart={(event) => { touchY.current = event.touches[0]?.clientY ?? 0; }}
        onTouchMove={(event) => {
          const y = event.touches[0]?.clientY ?? touchY.current;
          userScrollUntil.current = Date.now() + 500;
          if (y > touchY.current) followMessages.current = false;
          touchY.current = y;
        }}
        onPointerDown={(event) => {
          const pane = event.currentTarget;
          if (event.clientX >= pane.getBoundingClientRect().right - 20) {
            userScrollUntil.current = Date.now() + 30000;
          }
        }}
        onPointerUp={() => { if (userScrollUntil.current > Date.now() + 500) userScrollUntil.current = Date.now() + 500; }}
        onScroll={(event) => {
          const pane = event.currentTarget;
          const atBottom = pane.scrollHeight - pane.scrollTop - pane.clientHeight < 70;
          if (atBottom) followMessages.current = true;
          else if (Date.now() < userScrollUntil.current) followMessages.current = false;
        }}
      >
        <div ref={messageContent} className="chat-message-content">
        {messages.length === 0 && (
          <div className="chat-welcome">
            <div className="chat-welcome-planet">
              <Planet
                color={orbit?.color}
                name={orbit?.name}
                identity={orbit?.id}
                size={compact ? 90 : 128}
              />
            </div>
            <h3>
              {team.length > 1
                ? "A team for your next idea"
                : `Say hello to ${orbit?.name ?? "your Sprite"}`}
            </h3>
            <p>
              {team.length > 1
                ? "Share the goal. Your lead Sprite will bring in the right specialists."
                : orbit?.personality ||
                  "Start with a question, a task, or something you’re thinking about."}
            </p>
            <div className="suggestion-chips">
              {(team.length > 1
                ? [
                    "Help me plan a project",
                    "Research an idea together",
                    "Review our current work",
                  ]
                : ["What can you help me with?", "Help me plan my next steps"]
              ).map((suggestion) => (
                <button
                  key={suggestion}
                  onClick={() => {
                    setDraft(suggestion);
                    textarea.current?.focus();
                  }}
                >
                  {suggestion}
                </button>
              ))}
            </div>
          </div>
        )}
        {messages.map((message) => {
          const author =
            data.orbits.find((item) => item.id === message.orbitId) ?? orbit;
          if (message.role === "tool")
            return <ToolActivityPanel key={message.id} message={message} servers={data.mcpServers} />;
          if (message.role === "system")
            return (
              <div className="system-message" key={message.id}>
                {message.content}
              </div>
            );
          return (
            <article
              className={`message message-${message.role}`}
              key={message.id}
            >
              <div className="message-avatar">
                {message.role === "assistant" ? (
                  <Planet color={author?.color} name={author?.name} identity={author?.id} size={34} />
                ) : (
                  <span>You</span>
                )}
              </div>
              <div className="message-body">
                <header>
                  <strong>
                    {message.role === "assistant"
                      ? (author?.name ?? "Sprite")
                      : "You"}
                  </strong>
                  <time>
                    {new Date(message.createdAt).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </time>
                </header>
                <div className="markdown">
                  <ReactMarkdown
                    remarkPlugins={[remarkGfm]}
                    components={{
                      a: (props) => (
                        <a
                          {...props}
                          target="_blank"
                          rel="noreferrer noopener"
                          onClick={(event) => {
                            if (!native || !props.href) return;
                            event.preventDefault();
                            void api("open_external_url", { url: props.href }).catch((failure) => setError(errorText(failure)));
                          }}
                        />
                      ),
                    }}
                  >
                    {message.content}
                  </ReactMarkdown>
                </div>
              </div>
            </article>
          );
        })}
        {running.length > 0 && (
          <div className="thinking-indicator">
            <LoaderCircle size={14} className="spin" />
            <span>
              {running
                .map(
                  (task) =>
                    data.orbits.find((item) => item.id === task.orbitId)
                      ?.name ?? "Sprite",
                )
                .join(", ")}{" "}
              {approvals.length ? "waiting for your approval" : "working"}
            </span>
          </div>
        )}
        {approvals.map((approval) => (
          <div className="approval-card" key={approval.id}>
            <div className="approval-title">
              <ShieldCheck size={17} />
              <strong>
                {data.orbits.find((item) => item.id === approval.orbitId)?.name}{" "}
                would like permission
              </strong>
            </div>
            <ToolApprovalDetails name={approval.tool} args={approval.arguments} servers={data.mcpServers} />
            <div className="button-row">
              <button
                className="button button-quiet"
                onClick={() =>
                  void action("resolve_approval", {
                    id: approval.id,
                    allow: false,
                  })
                }
              >
                Decline
              </button>
              <button
                className="button button-primary"
                onClick={() =>
                  void action("resolve_approval", {
                    id: approval.id,
                    allow: true,
                  })
                }
              >
                <Check size={14} /> Allow once
              </button>
            </div>
          </div>
        ))}
        <div ref={bottom} />
        </div>
      </div>
      <div className="composer-region">
        {error && (
          <div className="inline-error" role="alert">
            {error}
            <button onClick={() => setError("")} aria-label="Dismiss error">
              ×
            </button>
          </div>
        )}
        {!connected && onConnections && (
          <div className="connection-hint">
            <span>
              Connect a model and assign it to {orbit?.name ?? "this Sprite"} to
              begin.
            </span>
            <button onClick={onConnections}>
              Set up connection <ArrowUp size={12} />
            </button>
          </div>
        )}
        {sharing && (
          <div className="sharing-banner">
            <Monitor size={14} />
            <span>Screen sharing is active for {orbit?.name}</span>
            <button disabled={sharingBusy} onClick={() => void toggleSharing()}>
              Stop sharing
            </button>
          </div>
        )}
        <form
          className="composer"
          onSubmit={(event) => {
            event.preventDefault();
            void send();
          }}
        >
          <textarea
            ref={textarea}
            rows={compact ? 2 : 3}
            aria-label="Message your Sprite"
            placeholder={
              team.length > 1
                ? "Tell your team what you have in mind…"
                : `Message ${orbit?.name ?? "your Sprite"}…`
            }
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (
                event.key === "Enter" &&
                !event.shiftKey &&
                !event.nativeEvent.isComposing
              ) {
                event.preventDefault();
                void send();
              }
            }}
          />
          <div className="composer-actions">
            <div className="button-row">
              <button
                type="button"
                className={`composer-tool ${sharing ? "active" : ""}`}
                disabled={sharingBusy}
                title={
                  sharing
                    ? "Stop screen sharing"
                    : "Share your screen for this task"
                }
                aria-label={sharing ? "Stop screen sharing" : "Share screen"}
                onClick={() => void toggleSharing()}
              >
                <Monitor size={17} />
                <span>Share screen</span>
              </button>
              {team.length > 1 && (
                <span className="composer-members">
                  <Users size={14} />
                  {team.length}
                </span>
              )}
            </div>
            {running.length ? (
              <button
                type="button"
                className="stop-button"
                aria-label="Stop active tasks"
                onClick={() =>
                  void Promise.all(
                    running.map((task) =>
                      action("stop_task", { taskId: task.id }),
                    ),
                  )
                }
              >
                <Square size={13} />
              </button>
            ) : (
              <button
                type="submit"
                className="send-button"
                disabled={!draft.trim() || sending}
                aria-label="Send message"
              >
                {sending ? (
                  <LoaderCircle size={17} className="spin" />
                ) : (
                  <ArrowUp size={19} />
                )}
              </button>
            )}
          </div>
        </form>
        <p className="composer-caption">
          {compact
            ? "Enter to send · Shift + Enter for a new line"
            : "All tools are available. Your approval preferences and project access still apply. Verify important work."}
        </p>
      </div>
      {settingsOpen && orbit && <OrbitSettingsPanel orbit={orbit} data={data}
        onClose={() => setSettingsOpen(false)} onChanged={(change) => { if (change !== "edit") { setSettingsOpen(false); onOrbitChanged?.(); } }} />}
    </section>
  );
}
