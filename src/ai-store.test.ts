import { describe, expect, it, vi } from "vitest";

// A request whose server never answers until it is stopped, like one still
// reading the database's structure or waiting for the provider's first word.
const calls: { onText: (text: string) => void; reject: (e: unknown) => void; stop: ReturnType<typeof vi.fn> }[] = [];
vi.mock("./lib/ai", async (original) => ({
  ...(await original<typeof import("./lib/ai")>()),
  ai: {
    ask: (_request: unknown, onText: (text: string) => void) => {
      let reject: (e: unknown) => void = () => {};
      const done = new Promise((_, no) => (reject = no));
      const stop = vi.fn();
      calls.push({ onText, reject, stop });
      return { done, stop };
    },
  },
}));

const { ask, stopAsking, isAsking, useAi } = await import("./ai-store");

describe("stopping an AI request", () => {
  it("ends the turn at once and ignores what arrives later", async () => {
    ask("tab", { request: "Explain", task: { kind: "explain" }, session: null, production: false });
    expect(isAsking("tab")).toBe(true);

    stopAsking("tab");
    expect(calls[0].stop).toHaveBeenCalled();
    expect(isAsking("tab")).toBe(false);
    expect(useAi.getState().threads.tab[0].state).toBe("stopped");

    calls[0].onText("late text");
    calls[0].reject({ kind: "ai", message: { reason: "cancelled", message: "stopped" } });
    await new Promise((r) => setTimeout(r, 0));
    expect(useAi.getState().threads.tab[0]).toMatchObject({ state: "stopped", text: "" });

    // A new request can start straight away.
    ask("tab", { request: "Again", task: { kind: "explain" }, session: null, production: false });
    expect(calls).toHaveLength(2);
    expect(isAsking("tab")).toBe(true);
  });
});
