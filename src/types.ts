export type Autonomy = "ask" | "workspace" | "trusted";
export type ProviderKind = "openai" | "litellm" | "chatgpt" | "claude";

export interface Orbit {
  id: string;
  name: string;
  role: string;
  personality: string;
  instructions: string;
  tasks?: string;
  color: string;
  model: string;
  providerId: string;
  autonomy: Autonomy;
  toolGrants: string[];
  projectIds: string[];
  x?: number;
  y?: number;
  enabled: boolean;
}

export interface Provider {
  id: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  models: string[];
  connected: boolean;
  experimental: boolean;
}

export interface Project {
  id: string;
  name: string;
  directory: string;
  description: string;
}

export interface Conversation {
  id: string;
  title: string;
  orbitIds: string[];
  leadOrbitId: string;
  projectId?: string;
  createdAt: string;
}

export interface Message {
  id: string;
  conversationId: string;
  orbitId?: string;
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  createdAt: string;
  toolActivity?: ToolActivity;
}

export interface ToolFileDiff {
  path: string;
  patch: string;
  truncated: boolean;
}

export interface ToolActivity {
  callId: string;
  taskId?: string;
  name: string;
  status: "running" | "completed" | "failed" | "denied" | "cancelled" | "interrupted";
  arguments: unknown;
  output: string;
  truncated: boolean;
  exitCode?: number;
  diffs: ToolFileDiff[];
}

export interface Task {
  id: string;
  conversationId: string;
  orbitId: string;
  status: string;
  summary: string;
  error?: string;
  createdAt: string;
}

export interface Approval {
  id: string;
  taskId: string;
  orbitId: string;
  tool: string;
  arguments: unknown;
}

export interface AttentionItem {
  id: string;
  orbitId: string;
  taskId: string;
  conversationId: string;
  kind: "approval" | "failed" | "message";
  title: string;
  createdAt: string;
}

export interface AttentionState {
  items: AttentionItem[];
  current?: AttentionItem;
  geometry: { width: number; height: number; notchWidth: number; notchHeight: number; mode: "notch" | "edge" };
  reducedMotion: boolean;
  soundEnabled: boolean;
}

export interface Memory {
  id: string;
  orbitIds?: string[];
  projectIds?: string[];
  orbitId?: string;
  projectId?: string;
  content: string;
  createdAt: string;
}

export interface Routine {
  id: string;
  title?: string;
  cron?: string;
  orbitId: string;
  prompt: string;
  intervalMinutes: number;
  enabled: boolean;
  projectId?: string;
  trigger: "cron" | "interval" | "folder" | "completion";
  lastRunAt?: string;
  observedSignature?: string;
}

export interface Settings {
  reducedMotion?: boolean;
  desktopEnabled?: boolean;
  attentionSound?: boolean;
  attentionVolume?: number;
  onboardingComplete?: boolean;
  onboardingStep?: string;
  soundEffects?: boolean;
  soundEffectsVolume?: number;
  [key: string]: unknown;
}

export interface McpServer {
  id: string;
  name: string;
  transport: "stdio" | "http";
  command?: string;
  args?: string[];
  url?: string;
  enabled: boolean;
  connected?: boolean;
  hasCredentials?: boolean;
}

export interface Bootstrap {
  orbits: Orbit[];
  providers: Provider[];
  projects: Project[];
  conversations: Conversation[];
  routines: Routine[];
  tasks: Task[];
  approvals: Approval[];
  settings: Settings;
  mcpServers?: McpServer[];
  sharingOrbitIds?: string[];
  desktopExpanded?: boolean;
  attentionItems?: AttentionItem[];
}

export const emptyBootstrap: Bootstrap = {
  orbits: [],
  providers: [],
  projects: [],
  conversations: [],
  routines: [],
  tasks: [],
  approvals: [],
  settings: {},
};

export const tools = [
  {
    id: "files",
    name: "Files",
    description: "Read and edit granted project files",
  },
  {
    id: "web",
    name: "Web",
    description: "Retrieve web pages and research sources",
  },
  {
    id: "browser",
    name: "Browser",
    description: "Navigate and inspect a dedicated browser",
  },
  {
    id: "shell",
    name: "Shell",
    description: "Run approved commands with host access",
  },
  {
    id: "screen",
    name: "Screen sharing",
    description: "View a task-scoped screen share",
  },
  {
    id: "computer",
    name: "Computer use",
    description: "Interact with approved applications",
  },
  { id: "mcp", name: "MCP tools", description: "Use connected tool servers" },
  {
    id: "collaboration",
    name: "Collaboration",
    description: "Request help from other Orbits",
  },
];

export const activeTask = (task: Task) =>
  ["queued", "running", "awaiting_approval", "waiting", "working"].includes(
    task.status,
  );
export const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export function isProviderConfigured(provider: Provider) {
  return (
    provider.connected ||
    (["openai", "litellm"].includes(provider.kind) && Boolean(provider.baseUrl))
  );
}
