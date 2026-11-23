// Canned assistant for a development build opened in a plain browser, so the
// AI screens can be reviewed without a key. Release builds never load it.
import type { AiBackend, Completion, ProviderEntry } from "./ai";

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let providers: ProviderEntry[] = [
  { id: "p1", kind: "anthropic", name: "Anthropic", model: "claude-opus-5-5", has_key: true, is_default: true },
  { id: "p2", kind: "open_ai_compatible", name: "Ollama", base_url: "http://localhost:11434/v1", model: "qwen3-coder", has_key: false, is_default: false },
];

const REVIEW = `Verdict: Warnings

- WARNING line 1: \`SELECT\` with \`*\` reads every column; name the ones you need.
- WARNING line 3: there is no LIMIT, and \`orders\` has about 8.4M rows.
- NOTE line 2: \`status\` is compared with text; an enum would catch typos.

Suggested query:
\`\`\`sql
SELECT o.id, o.user_id, o.total_cents, o.created_at
FROM orders o
WHERE o.status = 'paid'
ORDER BY o.created_at DESC
LIMIT 100;
\`\`\`

Suggested index:
\`\`\`sql
CREATE INDEX orders_status_created_at ON orders (status, created_at DESC);
\`\`\`
It serves the filter and the order together. Building it on a large table takes time and can slow writes while it runs.`;

const CHAT = `Here is revenue per country for the last 30 days. I can't see rows, so check the status values first.

\`\`\`sql
SELECT u.country, SUM(o.total_cents) / 100.0 AS revenue
FROM orders o
JOIN users u ON u.id = o.user_id
WHERE o.status = 'paid' AND o.created_at >= now() - interval '30 days'
GROUP BY u.country
ORDER BY revenue DESC;
\`\`\``;

// A first chat request is answered with a question, as an ambiguous one would be (AI14).
const QUESTION = `Question: Which date should decide the last 30 days?
- Option: orders.created_at — when the order was placed
- Option: orders.paid_at — when it was paid`;

export const demoAi: AiBackend = {
  listProviders: async () => providers,
  saveProvider: async (provider, apiKey) => {
    const existing = providers.find((p) => p.id === provider.id);
    const entry: ProviderEntry = { ...provider, id: provider.id || `p${Date.now()}`, has_key: apiKey === undefined ? (existing?.has_key ?? false) : apiKey !== "", is_default: existing?.is_default ?? providers.length === 0 };
    providers = existing ? providers.map((p) => (p.id === entry.id ? entry : p)) : [...providers, entry];
    return entry;
  },
  deleteProvider: async (id) => {
    providers = providers.filter((p) => p.id !== id);
  },
  setDefaultProvider: async (id) => {
    providers = providers.map((p) => ({ ...p, is_default: p.id === id }));
  },
  listModels: async () => [
    { id: "claude-opus-5-5", name: "Claude Opus 5.5" },
    { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" },
  ],
  // The demo collection holds four values per vector; the words decide them, so equal text finds the same points.
  embed: async (_id, _model, text) => {
    await wait(300);
    const sum = [...text].reduce((n, c) => n + c.charCodeAt(0), 0);
    return [0, 1, 2, 3].map((i) => Math.sin(sum + i));
  },
  testProvider: async (id) => {
    await wait(500);
    const provider = providers.find((p) => p.id === id);
    if (provider && !provider.has_key && provider.kind !== "open_ai_compatible") throw { kind: "ai", message: { reason: "auth", message: "401 · Incorrect API key provided" } };
    return { text: "ok", model: provider?.model ?? "claude-opus-5-5", stop: "end", usage: { input_tokens: 12, output_tokens: 2 } };
  },
  previewContext: async () => ({
    text: "orders (≈ 8.4M rows)\n  id int8 PK\n  user_id int8 → users.id\n  status text\n  total_cents int4\n  created_at timestamptz\n  index orders_pkey (id) unique\n\nusers (≈ 1.2M rows)\n  id int8 PK\n  email text\n  country bpchar",
    namespace: "public",
    tables_total: 12,
    tables_detailed: 2,
  }),
  ask(request, onText) {
    let stopped = false;
    const chat = request.history?.length ? CHAT : QUESTION;
    const reply = request.task.kind === "ask" ? chat : request.task.kind === "explain" ? "It reads paid orders, newest first. Nothing is changed." : REVIEW;
    const done = (async (): Promise<Completion> => {
      if (providers.length === 0) throw { kind: "ai", message: { reason: "not_configured", message: "No AI provider is set up." } };
      for (let i = 0; i < reply.length; i += 24) {
        await wait(40);
        if (stopped) throw { kind: "ai", message: { reason: "cancelled", message: "Stopped." } };
        onText(reply.slice(i, i + 24));
      }
      return { text: reply, model: "claude-opus-5-5", stop: "end", usage: { input_tokens: 1840, output_tokens: 212 } };
    })();
    return { done, stop: () => (stopped = true) };
  },
};
