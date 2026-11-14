import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reportDiagnostic } from "./diagnostics";
import { orbitProfileNeedsGeneration } from "./components/orbitProfile";
import type {
  AttentionItem,
  AttentionState,
  Bootstrap,
  Conversation,
  Memory,
  McpServer,
  Message,
  Orbit,
  Provider,
  Settings,
} from "./types";

export const native = isTauri();
const previewKey = "solar.preview.v1";
const previewEvent = "solar-preview-change";

interface PreviewState extends Bootstrap {
  messages: Message[];
  memories: Memory[];
}

const now = () => new Date().toISOString();

function initialPreview(): PreviewState {
  return {
    orbits: [],
    providers: [],
    projects: [],
    conversations: [],
    routines: [],
    tasks: [],
    approvals: [],
    memories: [],
    messages: [],
    mcpServers: [],
    sharingOrbitIds: [],
    settings: {
      desktopEnabled: true,
      reducedMotion: false,
      maxConcurrent: 2,
      experimentalClaude: false,
      onboardingComplete: false,
      onboardingStep: "birth",
      soundEffects: true,
      soundEffectsVolume: 0.25,
    },
  };
}

function readPreview(): PreviewState {
  try {
    const stored = localStorage.getItem(previewKey);
    if (stored)
      return { ...initialPreview(), ...JSON.parse(stored) } as PreviewState;
  } catch {
    /* A blocked or unavailable localStorage still allows the preview to run. */
  }
  return initialPreview();
}

let preview = readPreview();
let previewAttention: AttentionItem[] = [];

function previewAttentionState(): AttentionState {
  return {
    items: structuredClone(previewAttention),
    current: previewAttention[0] ? structuredClone(previewAttention[0]) : undefined,
    geometry: { width: 454, height: 230, notchWidth: 0, notchHeight: 0, mode: "edge" },
    reducedMotion: Boolean(preview.settings.reducedMotion),
    soundEnabled: false,
  };
}

function emitPreviewAttention() {
  window.dispatchEvent(new CustomEvent("solar-preview-attention", {
    detail: previewAttentionState(),
  }));
  window.dispatchEvent(new Event(previewEvent));
}

function savePreview() {
  try {
    localStorage.setItem(previewKey, JSON.stringify(preview));
  } catch {
    /* Private browser sessions can disable storage. */
  }
  window.dispatchEvent(new Event(previewEvent));
}

function replaceById<T extends { id: string }>(items: T[], item: T): T[] {
  return items.some((existing) => existing.id === item.id)
    ? items.map((existing) => (existing.id === item.id ? item : existing))
    : [...items, item];
}

function requiredString(args: Record<string, unknown>, key: string) {
  const value = args[key];
  if (typeof value !== "string") throw new Error(`Missing ${key}`);
  return value;
}

// Browser preview simulates generation locally; native profiles use the selected model.
function previewOrbitProfile(draft: Orbit, existing?: Orbit): Orbit {
  if (existing && !orbitProfileNeedsGeneration(draft, existing)) {
    return { ...draft, instructions: existing.instructions };
  }
  const tasks = (draft.tasks ?? "").trim();
  if (!draft.name.trim() || !draft.personality.trim() || !tasks) throw new Error("Name, personality, and tasks are required.");
  if (!draft.model || !preview.providers.some((item) => item.id === draft.providerId)) throw new Error("Choose a model for your Orbit.");
  const name = draft.name.trim();
  const gender = draft.gender?.trim() ?? "";
  const personality = draft.personality.trim();
  const role = existing ? draft.role.trim() : "Personal Orbit";
  if (!role) throw new Error("Role is required.");
  return {
    ...draft, name, gender, personality, tasks, role, toolGrants: ["*"],
    instructions: [
      `You are ${name}, a ${role} in Solar.`,
      gender && `Gender: ${gender}.`,
      `Personality: ${personality}`,
      `Responsibilities: ${tasks}`,
      "Use available tools to complete the user's request. Gather evidence, respect their boundaries, and report outcomes clearly.",
    ].filter(Boolean).join("\n\n"),
  };
}

async function previewCommand(
  command: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  switch (command) {
    case "create_orbit": {
      const draft = args.orbit as Orbit;
      if (preview.orbits.some((item) => item.id === draft.id)) throw new Error("This Orbit already exists.");
      const orbit = previewOrbitProfile(draft);
      preview.orbits = [...preview.orbits, orbit];
      savePreview();
      return orbit;
    }
    case "update_orbit_profile": {
      const draft = args.orbit as Orbit;
      const existing = preview.orbits.find((item) => item.id === draft.id);
      if (!existing) throw new Error("This Orbit no longer exists.");
      const orbit = previewOrbitProfile(draft, existing);
      preview.orbits = replaceById(preview.orbits, orbit);
      savePreview();
      return orbit;
    }
    case "cancel_orbit_prompt_analysis":
      return;
    case "get_bootstrap":
      return { ...structuredClone(preview), attentionItems: structuredClone(previewAttention) };
    case "save_onboarding_progress": {
      const step = requiredString(args, "step");
      if (!["birth", "welcome", "team", "desktop", "connection", "orbit", "workspace", "ready"].includes(step)) throw new Error("Unknown welcome step");
      preview.settings = { ...preview.settings, onboardingStep: step };
      savePreview();
      return preview.settings;
    }
    case "complete_onboarding":
      preview.settings = { ...preview.settings, onboardingComplete: true, onboardingStep: "ready" };
      savePreview();
      return preview.settings;
    case "play_sound_effect":
      return false;
    case "stop_sound_effects":
    case "set_intro_fullscreen":
      return;
    case "reset_solar":
      if (args.confirmed !== true) throw new Error("Confirm Reset Solar before continuing.");
      preview = initialPreview();
      previewAttention = [];
      savePreview();
      emitPreviewAttention();
      return;
    case "get_attention_state":
      return previewAttentionState();
    case "attention_ready":
    case "attention_bounds":
      return;
    case "preview_attention": {
      const orbit = preview.orbits.find((item) => item.id === args.orbitId && item.enabled);
      if (!orbit) throw new Error("Choose an enabled Orbit to preview.");
      const conversation = preview.conversations.find((item) =>
        item.orbitIds.length === 1 && item.orbitIds[0] === orbit.id,
      );
      previewAttention.push({
        id: crypto.randomUUID(),
        orbitId: orbit.id,
        taskId: "preview",
        conversationId: conversation?.id ?? "preview",
        kind: "message",
        title: "Your update is ready",
        createdAt: now(),
      });
      emitPreviewAttention();
      window.location.hash = "attention";
      return;
    }
    case "dismiss_attention":
    case "open_attention": {
      const item = previewAttention.find((attention) => attention.id === args.id);
      if (!item) throw new Error("This attention request is no longer waiting.");
      previewAttention = previewAttention.filter((attention) => attention.id !== item.id);
      emitPreviewAttention();
      if (command === "open_attention") window.location.hash = `chat/${item.orbitId}`;
      else if (!previewAttention.length) window.location.hash = "";
      return;
    }
    case "get_messages":
      return preview.messages.filter(
        (message) => message.conversationId === args.conversationId,
      );
    case "save_orbit": {
      const orbit = args.orbit as Orbit;
      preview.orbits = replaceById(preview.orbits, orbit);
      savePreview();
      return orbit;
    }
    case "reset_orbit": {
      if (args.confirmed !== true) throw new Error("Confirm before clearing this Orbit's saved data.");
      const privateIds = new Set(preview.conversations.filter((chat) => chat.orbitIds.length === 1 && chat.orbitIds[0] === args.orbitId).map((chat) => chat.id));
      preview.conversations = preview.conversations.filter((chat) => !privateIds.has(chat.id));
      preview.messages = preview.messages.filter((message) => !privateIds.has(message.conversationId));
      preview.memories = preview.memories.filter((memory) => memory.orbitId !== args.orbitId);
      preview.tasks = preview.tasks.filter((task) => task.orbitId !== args.orbitId);
      previewAttention = previewAttention.filter((item) => item.orbitId !== args.orbitId);
      savePreview();
      emitPreviewAttention();
      return;
    }
    case "delete_orbit":
      preview.orbits = preview.orbits.filter((orbit) => orbit.id !== args.id);
      previewAttention = previewAttention.filter((item) => item.orbitId !== args.id);
      if ([`chat/${String(args.id)}`, `orbit/${String(args.id)}`].includes(window.location.hash.slice(1))) window.location.hash = "";
      savePreview();
      emitPreviewAttention();
      return;
    case "save_provider": {
      const input = args.provider as Provider & { apiKey?: string };
      const provider: Provider = {
        id: input.id,
        name: input.name,
        kind: input.kind,
        baseUrl: input.baseUrl,
        models: input.models,
        connected: false,
        experimental: input.experimental,
      };
      preview.providers = replaceById(preview.providers, provider);
      savePreview();
      return provider;
    }
    case "discover_models":
    case "start_login":
      throw new Error(
        "Provider connections run in the native Solar app. This browser is a UI preview.",
      );
    case "logout_provider":
      preview.providers = preview.providers.map((provider) =>
        provider.id === args.id ? { ...provider, connected: false } : provider,
      );
      savePreview();
      return;
    case "create_conversation": {
      const conversation = {
        ...args,
        id: crypto.randomUUID(),
        createdAt: now(),
      } as unknown as Conversation;
      preview.conversations = [conversation, ...preview.conversations];
      savePreview();
      return conversation;
    }
    case "send_message": {
      const conversationId = requiredString(args, "conversationId");
      const content = requiredString(args, "content");
      const conversation = preview.conversations.find(
        (item) => item.id === conversationId,
      );
      preview.messages.push({
        id: crypto.randomUUID(),
        conversationId,
        role: "user",
        content,
        createdAt: now(),
      });
      preview.messages.push({
        id: crypto.randomUUID(),
        conversationId,
        orbitId: conversation?.leadOrbitId,
        role: "system",
        content:
          "UI preview — this message was saved locally. Open the native Solar app and connect a model to receive a real response.",
        createdAt: now(),
      });
      savePreview();
      return;
    }
    case "save_project":
      preview.projects = replaceById(
        preview.projects,
        args.project as PreviewState["projects"][number],
      );
      savePreview();
      return args.project;
    case "save_routine":
      preview.routines = replaceById(
        preview.routines,
        args.routine as PreviewState["routines"][number],
      );
      savePreview();
      return args.routine;
    case "get_memories":
      return preview.memories.filter((memory) =>
        args.orbitId
          ? memory.orbitId === args.orbitId
          : args.projectId
            ? memory.projectId === args.projectId
            : true,
      );
    case "save_memory":
      preview.memories = replaceById(preview.memories, args.memory as Memory);
      savePreview();
      return args.memory;
    case "delete_memory":
      preview.memories = preview.memories.filter(
        (memory) => memory.id !== args.id,
      );
      savePreview();
      return;
    case "save_settings":
      preview.settings = {
        ...preview.settings,
        ...(args.settings as Settings),
        onboardingStep: preview.settings.onboardingStep,
        onboardingComplete: preview.settings.onboardingComplete,
      };
      savePreview();
      emitPreviewAttention();
      return preview.settings;
    case "save_mcp_server": {
      const input = args.server as McpServer;
      const server: McpServer = {
        id: input.id,
        name: input.name,
        transport: input.transport,
        command: input.command,
        args: input.args,
        url: input.url,
        enabled: input.enabled,
        connected: false,
        hasCredentials: false,
      };
      preview.mcpServers = replaceById(preview.mcpServers ?? [], server);
      savePreview();
      return;
    }
    case "connect_mcp":
      throw new Error("MCP connections run in the native Solar app.");
    case "delete_mcp_server":
      preview.mcpServers = preview.mcpServers?.filter(
        (server) => server.id !== args.id,
      );
      savePreview();
      return;
    case "save_position":
      preview.orbits = preview.orbits.map((orbit) =>
        orbit.id === args.orbitId
          ? { ...orbit, x: Number(args.x), y: Number(args.y) }
          : orbit,
      );
      savePreview();
      return;
    case "export_orbit":
      return JSON.stringify(
        {
          schemaVersion: 1,
          orbit: preview.orbits.find((orbit) => orbit.id === args.id),
        },
        null,
        2,
      );
    case "import_orbit": {
      const data = JSON.parse(requiredString(args, "profileJson")) as {
        orbit: Orbit;
      };
      if (!data.orbit?.name || !data.orbit?.role)
        throw new Error("Choose an exported Solar Orbit profile.");
      const orbit = {
        ...data.orbit,
        id: crypto.randomUUID(),
        providerId: "",
        projectIds: [],
        enabled: true,
      };
      preview.orbits.push(orbit);
      savePreview();
      return orbit;
    }
    case "set_desktop_expanded":
    case "set_active_chat":
      return;
    case "open_orbit_chat":
      window.location.hash = `chat/${String(args.orbitId)}`;
      return;
    case "open_orbit_settings":
      sessionStorage.setItem("solar.preview.edit-orbit", String(args.orbitId));
      window.location.hash = "";
      window.dispatchEvent(new CustomEvent("solar-preview-edit-orbit", { detail: { orbitId: args.orbitId } }));
      return;
    case "open_launcher":
      window.location.hash = "";
      return;
    case "start_screen_share":
      throw new Error(
        "Screen sharing requires the native Solar app and macOS permission.",
      );
    case "stop_screen_share":
      return;
    case "stop_task":
    case "resolve_approval":
      return;
    default:
      throw new Error(`The preview does not implement ${command}.`);
  }
}

export async function api<T = void>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (native) {
    try {
      return await invoke<T>(command, args);
    } catch (error) {
      reportDiagnostic("command_failed", error, { command });
      throw error;
    }
  }
  return previewCommand(command, args) as Promise<T>;
}

export async function subscribe(onChange: () => void): Promise<() => void> {
  if (!native) {
    window.addEventListener(previewEvent, onChange);
    return () => window.removeEventListener(previewEvent, onChange);
  }
  const cleanup = await Promise.all(
    [
      "solar://state",
      "solar://message",
      "solar://task",
      "solar://approval",
      "solar://attention",
    ].map((event) => listen(event, onChange)),
  );
  return () => cleanup.forEach((unlisten) => unlisten());
}
