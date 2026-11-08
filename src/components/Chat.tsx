import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowUp,
  Check,
  ChevronDown,
  Expand,
  LoaderCircle,
  Monitor,
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

let presenceRevision = Date.now() * 1_000;
const nextPresenceRevision = () => (presenceRevision = Math.max(presenceRevision + 1, Date.now() * 1_000));

export function Chat({
  conversation,
  data,
  onBack,
  compact = false,
  onExpand,
  onConnections,
}: {
  conversation: Conversation;
  data: Bootstrap;
  onBack?: () => void;
  compact?: boolean;
  onExpand?: () => void;
  onConnections?: () => void;
}) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sharingBusy, setSharingBusy] = useState(false);
  const [error, setError] = useState("");
  const bottom = useRef<HTMLDivElement>(null);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const currentRequest = useRef(0);
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
        if (sequence === currentRequest.current) setMessages(result);
      })
      .catch((failure) => {
        if (sequence === currentRequest.current) setError(errorText(failure));
      });
  }, [conversation.id, data]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: "instant", block: "end" });
  }, [messages.length, messages.at(-1)?.content]);
  useEffect(() => {
    setDraft("");
    setError("");
  }, [conversation.id]);

  async function send() {
    if (!draft.trim() || sending) return;
    const content = draft.trim();
    setSending(true);
    setError("");
    try {
      await api("send_message", { conversationId: conversation.id, content });
      setDraft("");
      setMessages(
        await api<Message[]>("get_messages", {
          conversationId: conversation.id,
        }),
      );
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
      <header className="chat-header">
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
              color={item.color}
              size={42}
            />
          ))}
        </div>
        <div className="chat-heading">
          <h2>{conversation.title}</h2>
          <p>
            {team.length > 1
              ? `${team.map((item) => item.name).join(", ")} · ${orbit?.name ?? "Orbit"} leads`
              : (orbit?.role ?? "Orbit conversation")}
          </p>
        </div>
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
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
      >
        {messages.length === 0 && (
          <div className="chat-welcome">
            <div className="chat-welcome-planet">
              <Planet
                color={orbit?.color}
                name={orbit?.name}
                size={compact ? 90 : 128}
              />
            </div>
            <h3>
              {team.length > 1
                ? "A team for your next idea"
                : `Say hello to ${orbit?.name ?? "your Orbit"}`}
            </h3>
            <p>
              {team.length > 1
                ? "Share the goal. Your lead Orbit will bring in the right specialists."
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
            return (
              <details className="tool-result" key={message.id}>
                <summary>
                  <ShieldCheck size={14} /> Tool result{" "}
                  <ChevronDown size={13} />
                </summary>
                <pre>{message.content}</pre>
              </details>
            );
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
                  <Planet color={author?.color} name={author?.name} size={34} />
                ) : (
                  <span>You</span>
                )}
              </div>
              <div className="message-body">
                <header>
                  <strong>
                    {message.role === "assistant"
                      ? (author?.name ?? "Orbit")
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
                      ?.name ?? "Orbit",
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
                wants to use {approval.tool}
              </strong>
            </div>
            <pre>{JSON.stringify(approval.arguments, null, 2)}</pre>
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
              Connect a model and assign it to {orbit?.name ?? "this Orbit"} to
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
            aria-label="Message your Orbit"
            placeholder={
              team.length > 1
                ? "Tell your team what you have in mind…"
                : `Message ${orbit?.name ?? "your Orbit"}…`
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
            : "Orbits use only the tools and shared context you grant them. Verify important work."}
        </p>
      </div>
    </section>
  );
}
