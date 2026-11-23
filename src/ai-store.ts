// State of the AI assistant: providers, privacy choices, the Ask AI panel of
// each query tab and the conversation of each chat tab.
import { create } from "zustand";
import { ai, aiError, type AiError, type ChatMessage, type Completion, type ProviderEntry, type Task } from "./lib/ai";

/** The part of the editor text a panel request is about. */
export interface Scope {
  kind: "statement" | "selection";
  from: number;
  to: number;
  fromLine: number;
  toLine: number;
  sql: string;
}

export interface Turn {
  id: number;
  /** What the user asked, as shown under YOU. */
  request: string;
  task: Task;
  state: "streaming" | "done" | "stopped" | "error";
  text: string;
  error: AiError | null;
  usage: Completion["usage"] | null;
  model: string | null;
  startedAt: number;
  endedAt: number | null;
  scope: Scope | null;
  /** The suggestion block was dismissed. */
  dismissed: boolean;
  /** Set once the suggestion was written into the editor: the text it replaced, for undo. */
  applied: { previous: string; lines: string } | null;
}

export interface Privacy {
  sendSchema: boolean;
  /** On Production connections. */
  productionSendSchema: boolean;
}

interface AiState {
  providers: ProviderEntry[];
  loaded: boolean;
  /** The provider requests go to; null follows the default. */
  providerId: string | null;
  privacy: Privacy;
  /** Turns by thread: a query tab's panel or a chat tab, keyed by tab id. */
  threads: Record<string, Turn[]>;
  /** Query tabs whose Ask AI panel is open. */
  open: Record<string, boolean>;
  panelWidth: number;
}

const PRIVACY_KEY = "lewide.ai.privacy";
function storedPrivacy(): Privacy {
  try {
    return { sendSchema: true, productionSendSchema: true, ...JSON.parse(localStorage.getItem(PRIVACY_KEY) ?? "{}") };
  } catch {
    return { sendSchema: true, productionSendSchema: true };
  }
}

export const useAi = create<AiState>(() => ({ providers: [], loaded: false, providerId: null, privacy: storedPrivacy(), threads: {}, open: {}, panelWidth: 380 }));
const { getState: get, setState: set } = useAi;

/** A provider can be asked once it has a key; an own endpoint may need none. */
export const usable = (p: ProviderEntry) => p.has_key || p.kind === "open_ai_compatible";

export function activeProvider(state: AiState = get()): ProviderEntry | null {
  const ready = state.providers.filter(usable);
  return ready.find((p) => p.id === state.providerId) ?? ready.find((p) => p.is_default) ?? ready[0] ?? null;
}

export async function loadProviders() {
  try {
    set({ providers: await ai.listProviders(), loaded: true });
  } catch {
    set({ loaded: true });
  }
}

export const chooseProvider = (providerId: string) => set({ providerId });
export const togglePanel = (tabId: string, open = !get().open[tabId]) => set((s) => ({ open: { ...s.open, [tabId]: open } }));
export const setPanelWidth = (width: number) => set({ panelWidth: Math.min(560, Math.max(320, width)) });

export function setPrivacy(patch: Partial<Privacy>) {
  const privacy = { ...get().privacy, ...patch };
  set({ privacy });
  try {
    localStorage.setItem(PRIVACY_KEY, JSON.stringify(privacy));
  } catch {
    // Without storage the choice lasts for this run.
  }
}

/** Whether the database's structure goes with a request on this kind of connection. */
export const sendsSchema = (production: boolean, privacy: Privacy = get().privacy) => privacy.sendSchema && (!production || privacy.productionSendSchema);

function patchTurn(key: string, id: number, patch: Partial<Turn>) {
  set((s) => ({ threads: { ...s.threads, [key]: (s.threads[key] ?? []).map((turn) => (turn.id === id ? { ...turn, ...patch } : turn)) } }));
}
export const updateTurn = patchTurn;
export const clearThread = (key: string) => set((s) => ({ threads: { ...s.threads, [key]: [] } }));

const running = new Map<string, () => void>();
let nextTurn = 1;

export interface AskInput {
  request: string;
  task: Task;
  scope?: Scope | null;
  sql?: string;
  session: number | null;
  engine?: string;
  namespace?: string;
  production: boolean;
  /** Send the earlier turns of the thread, for a conversation. */
  conversation?: boolean;
}

/** Sends a request and streams the answer into a new turn of the thread. */
export function ask(key: string, input: AskInput) {
  if (running.has(key)) return;
  const provider = activeProvider();
  const id = nextTurn++;
  const earlier = get().threads[key] ?? [];
  const history: ChatMessage[] = input.conversation
    ? earlier.filter((turn) => turn.state === "done" || turn.state === "stopped").flatMap((turn) => [{ role: "user" as const, content: turn.request }, { role: "assistant" as const, content: turn.text }])
    : [];
  const turn: Turn = { id, request: input.request, task: input.task, state: "streaming", text: "", error: null, usage: null, model: provider?.model ?? null, startedAt: Date.now(), endedAt: null, scope: input.scope ?? null, dismissed: false, applied: null };
  set((s) => ({ threads: { ...s.threads, [key]: [...earlier, turn] } }));

  const call = ai.ask(
    {
      session: input.session !== null && sendsSchema(input.production) ? input.session : undefined,
      engine: input.engine,
      namespace: input.namespace,
      providerId: provider?.id,
      task: input.task,
      sql: input.sql ?? input.scope?.sql,
      history,
    },
    (text) => {
      const current = (get().threads[key] ?? []).find((x) => x.id === id);
      if (current) patchTurn(key, id, { text: current.text + text });
    },
  );
  running.set(key, call.stop);
  call.done
    .then((completion) => patchTurn(key, id, { state: "done", text: completion.text, usage: completion.usage, model: completion.model, endedAt: Date.now() }))
    .catch((e) => {
      const error = aiError(e);
      // Stopping keeps what had arrived.
      if (error.reason === "cancelled") patchTurn(key, id, { state: "stopped", endedAt: Date.now() });
      else patchTurn(key, id, { state: "error", error, endedAt: Date.now() });
    })
    .finally(() => running.delete(key));
}

export const stopAsking = (key: string) => running.get(key)?.();
export const isAsking = (key: string, state: AiState = get()) => (state.threads[key] ?? []).some((turn) => turn.state === "streaming");
