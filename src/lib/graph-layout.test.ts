import { describe, expect, it } from "vitest";
import { caption, fit, labelLetters, layout } from "./graph-layout";

const distance = (a: { x: number; y: number }, b: { x: number; y: number }) => Math.hypot(a.x - b.x, a.y - b.y);

describe("graph layout", () => {
  const nodes = ["a", "b", "c", "d", "e", "f"];
  const edges: [string, string][] = [["a", "b"], ["b", "c"], ["a", "c"], ["d", "e"]];

  it("gives every node a finite place, apart from the others, the same way each time", () => {
    const first = layout(nodes, edges);
    expect(layout(nodes, edges)).toEqual(first);
    for (const a of nodes) {
      expect(Number.isFinite(first[a].x) && Number.isFinite(first[a].y)).toBe(true);
      for (const b of nodes) if (a !== b) expect(distance(first[a], first[b])).toBeGreaterThan(30);
    }
    // Linked nodes end up nearer than unlinked ones.
    expect(distance(first.a, first.b)).toBeLessThan(distance(first.a, first.d));
  });

  it("leaves fixed nodes where they are and puts new ones near the given point", () => {
    const fixed = { a: { x: 500, y: 500 }, b: { x: 600, y: 500 } };
    const next = layout(["a", "b", "n1", "n2"], [["a", "n1"], ["a", "n2"]], fixed, fixed.a);
    expect(next.a).toEqual(fixed.a);
    expect(next.b).toEqual(fixed.b);
    expect(distance(next.n1, fixed.a)).toBeLessThan(400);
    expect(distance(next.n1, next.n2)).toBeGreaterThan(30);
  });

  it("fits positions into a box and names nodes", () => {
    const view = fit([{ x: 0, y: 0 }, { x: 1000, y: 500 }], 800, 600);
    expect(view.k).toBeCloseTo(0.68);
    expect(view.x + 500 * view.k).toBeCloseTo(400);
    expect(fit([], 800, 600)).toEqual({ x: 400, y: 300, k: 1 });
    expect(caption({ title: "T", name: "N" })).toBe("N");
    expect(caption({ id: 7 })).toBe("7");
    expect(caption({ other: true })).toBe("");
  });

  it("gives each label its own letter", () => {
    expect(labelLetters(["Person", "Company", "City", "Country", "C"])).toEqual({ Person: "P", Company: "C", City: "I", Country: "O", C: "1" });
    expect(labelLetters(["", "Ürün"])).toEqual({ "": "·", Ürün: "Ü" });
  });
});
