// The AI assistant's commands. Provider requests are made by the Rust core
// (crates/ai), so an API key is only ever written here, never read back.
import { Channel, invoke } from "@tauri-apps/api/core";

export type ProviderKind = "anthropic" | "open_ai" | "gemini" | "open_ai_compatible";

export interface ProviderConfig {
  /** Empty for a provider that is being added. */
  id: string;
  kind: ProviderKind;
  name: string;
  /** Required for "open_ai_compatible" (Ollama, OpenRouter, a company gateway). */
  base_url?: string | null;
  /** Anthropic falls back to its default model; the others need one chosen. */
  model?: string | null;
}

export interface ProviderEntry extends ProviderConfig {
  has_key: boolean;
  is_default: boolean;
}

export interface ModelInfo {
  id: string;
  name: string;
}

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export type Task = { kind: "explain" } | { kind: "review" } | { kind: "optimize" } | { kind: "ask"; question: string };

/** What an AI call rejects with, read from the error. */
export interface AiError {
  reason: "not_configured" | "auth" | "rate_limit" | "model" | "request" | "refused" | "server" | "network" | "cancelled" | "other";
  message: string;
}

export function aiError(err: unknown): AiError {
  const e = err as { kind?: string; message?: unknown } | null;
  if (e && e.kind === "ai" && e.message && typeof e.message === "object") return e.message as AiError;
  const message = e && typeof e.message === "string" ? e.message : typeof err === "string" ? err : JSON.stringify(err);
  return { reason: "other", message };
}

export interface Completion {
  text: string;
  model: string;
  /** "length" means the reply was cut off at the output limit. */
  stop: "end" | "length" | "other";
  usage: { input_tokens: number | null; output_tokens: number | null };
}

/** What the assistant is told about the database: structure only, never rows. */
export interface SchemaContext {
  text: string;
  namespace: string | null;
  tables_total: number;
  tables_detailed: number;
}

export interface AskRequest {
  /** The connection the question is about; without one no structure is sent. */
  session?: number;
  /** Engine name as shown to the user, e.g. "PostgreSQL". */
  engine?: string;
  namespace?: string;
  /** Defaults to the default provider. */
  providerId?: string;
  task: Task;
  sql?: string;
  /** Earlier turns of the same conversation. */
  history?: ChatMessage[];
}

/**
 * Errors from these calls have `kind: "ai"` and `message: { reason, message }`,
 * where reason is one of: not_configured, auth, rate_limit, model, request,
 * refused, server, network, cancelled.
 */
const tauriAi = {
  listProviders: () => invoke<ProviderEntry[]>("ai_list_providers"),
  /** `apiKey`: undefined keeps the stored key, "" removes it, anything else replaces it. */
  saveProvider: (provider: ProviderConfig, apiKey?: string) =>
    invoke<ProviderEntry>("ai_save_provider", { provider, apiKey: apiKey ?? null }),
  deleteProvider: (id: string) => invoke<void>("ai_delete_provider", { id }),
  setDefaultProvider: (id: string) => invoke<void>("ai_set_default_provider", { id }),
  listModels: (id: string) => invoke<ModelInfo[]>("ai_list_models", { id }),
  /** Turns search text into a vector; only the text is sent. `model` falls back to the provider's usual embedding model. */
  embed: (id: string | null, model: string | null, text: string) => invoke<number[]>("ai_embed", { id, model, text }),
  /** Sends a one-line request to check the key, the address and the model. */
  testProvider: (id: string) => invoke<Completion>("ai_test_provider", { id }),
  previewContext: (session: number, namespace?: string, sql?: string) =>
    invoke<SchemaContext>("ai_preview_context", { session, namespace: namespace ?? null, sql: sql ?? null }),

  /**
   * Asks the assistant. `onText` receives the reply piece by piece; the
   * promise resolves with the whole reply. `stop()` ends it early, and the
   * promise then rejects with reason "cancelled".
   */
  ask(request: AskRequest, onText: (text: string) => void): { done: Promise<Completion>; stop(): void } {
    const requestId = crypto.randomUUID();
    const onEvent = new Channel<{ type: "text"; text: string }>();
    onEvent.onmessage = (event) => onText(event.text);
    const done = invoke<Completion>("ai_ask", {
      ask: {
        request_id: requestId,
        session: request.session ?? null,
        engine: request.engine ?? null,
        namespace: request.namespace ?? null,
        provider_id: request.providerId ?? null,
        task: request.task,
        sql: request.sql ?? null,
        history: request.history ?? [],
      },
      onEvent,
    });
    return { done, stop: () => void invoke("ai_cancel", { requestId }) };
  },
};

export type AiBackend = typeof tauriAi;

let backend: AiBackend = tauriAi;
/** Outside the Tauri shell a development build gets canned answers (see lib/demo-ai.ts). */
export const setAiBackend = (next: AiBackend) => {
  backend = next;
};

export const ai: AiBackend = {
  listProviders: () => backend.listProviders(),
  saveProvider: (provider, apiKey) => backend.saveProvider(provider, apiKey),
  deleteProvider: (id) => backend.deleteProvider(id),
  setDefaultProvider: (id) => backend.setDefaultProvider(id),
  listModels: (id) => backend.listModels(id),
  embed: (id, model, text) => backend.embed(id, model, text),
  testProvider: (id) => backend.testProvider(id),
  previewContext: (session, namespace, sql) => backend.previewContext(session, namespace, sql),
  ask: (request, onText) => backend.ask(request, onText),
};

/** A reply cut into prose and SQL blocks, in order, for drawing. */
export type Segment = { kind: "text"; text: string } | { kind: "sql"; sql: string; role: "suggestion" | "index" | "plain" };

export function segments(reply: string): Segment[] {
  const out: Segment[] = [];
  const block = /```(?:sql)?[ \t]*\n([\s\S]*?)```/gi;
  let last = 0;
  for (let match = block.exec(reply); match; match = block.exec(reply)) {
    let before = reply.slice(last, match.index);
    const label = before.trimEnd().split("\n").pop()?.trim().toLowerCase() ?? "";
    const role = label.startsWith("suggested query") ? "suggestion" : label.startsWith("suggested index") ? "index" : "plain";
    // The label line is the block's heading, not prose.
    if (role !== "plain") before = before.trimEnd().split("\n").slice(0, -1).join("\n");
    if (before.trim()) out.push({ kind: "text", text: before.trim() });
    out.push({ kind: "sql", sql: match[1].trim(), role });
    last = match.index + match[0].length;
  }
  const rest = reply.slice(last).trim();
  if (rest) out.push({ kind: "text", text: rest });
  return out;
}

/**
 * The statements in a reply, for "Apply" and "Copy". `suggested` is the one
 * the assistant marked as the replacement for the user's query, if any.
 */
export function sqlBlocks(reply: string): { all: string[]; suggested: string | null; indexes: string[] } {
  const all: string[] = [];
  let suggested: string | null = null;
  const indexes: string[] = [];
  const block = /```sql\s*\n([\s\S]*?)```/gi;
  for (let match = block.exec(reply); match; match = block.exec(reply)) {
    const sql = match[1].trim();
    all.push(sql);
    // The label is the last non-empty line before the block.
    const label = reply.slice(0, match.index).trimEnd().split("\n").pop()?.trim().toLowerCase() ?? "";
    if (label.startsWith("suggested query") && suggested === null) suggested = sql;
    if (label.startsWith("suggested index")) indexes.push(sql);
  }
  return { all, suggested, indexes };
}

/** The first line of a review, if the assistant followed the convention. */
export function verdict(reply: string): "ok" | "warnings" | "errors" | null {
  const match = /^\s*verdict:\s*(ok|warnings|errors)\b/i.exec(reply);
  return match ? (match[1].toLowerCase() as "ok" | "warnings" | "errors") : null;
}
