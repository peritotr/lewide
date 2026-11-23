import { describe, expect, it } from "vitest";
import { parsePlan, readPlan } from "./plan";

const explain = [
  {
    Plan: {
      "Node Type": "Limit", "Total Cost": 1000.5, "Plan Rows": 100,
      Plans: [
        {
          "Node Type": "Hash Join", "Hash Cond": "(o.user_id = u.id)", "Total Cost": 1000, "Plan Rows": 5000,
          Plans: [
            { "Node Type": "Seq Scan", "Relation Name": "orders", Alias: "o", Filter: "(status = 'paid')", "Total Cost": 800, "Plan Rows": 5000 },
            { "Node Type": "Index Scan", "Relation Name": "users", Alias: "users", "Index Name": "users_pkey", "Total Cost": 150, "Plan Rows": 300 },
          ],
        },
      ],
    },
  },
];

describe("parsePlan", () => {
  it("flattens the tree in display order with depth and connectors", () => {
    const plan = parsePlan(explain)!;
    expect(plan.totalCost).toBe(1000.5);
    expect(plan.nodes.map((n) => [n.depth, n.type, n.last])).toEqual([
      [0, "Limit", true],
      [1, "Hash Join", true],
      [2, "Seq Scan", false],
      [2, "Index Scan", true],
    ]);
    expect(plan.nodes[2].detail).toBe("on orders o · filter (status = 'paid')");
    expect(plan.nodes[3].detail).toBe("on users · using users_pkey");
  });

  it("charges each node only for its own work and marks the most expensive", () => {
    const plan = parsePlan(explain)!;
    expect(plan.nodes.map((n) => n.ownCost)).toEqual([0.5, 50, 800, 150]);
    expect(plan.nodes.filter((n) => n.mostExpensive).map((n) => n.type)).toEqual(["Seq Scan"]);
    expect(plan.nodes[2].share).toBeCloseTo(0.8, 2);
  });

  it("returns null for anything that is not a plan", () => {
    expect(parsePlan("Seq Scan on orders")).toBeNull();
    expect(parsePlan([{ rows: 1 }])).toBeNull();
    expect(parsePlan(null)).toBeNull();
  });
});

describe("readPlan", () => {
  it("points at a dominant full scan", () => {
    expect(readPlan(parsePlan(explain)!)).toContain("full scan on orders o");
  });

  it("says nothing when no node dominates", () => {
    const flat = [{ Plan: { "Node Type": "Append", "Total Cost": 100, Plans: [1, 2, 3, 4].map(() => ({ "Node Type": "Seq Scan", "Total Cost": 25 })) } }];
    expect(readPlan(parsePlan(flat)!)).toBeNull();
  });
});
