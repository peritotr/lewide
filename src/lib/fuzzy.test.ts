import { describe, expect, it } from "vitest";
import { fuzzy } from "./fuzzy";

describe("fuzzy match", () => {
  it("finds letters in order and marks them", () => {
    expect(fuzzy("ord", "orders")?.ranges).toEqual([[0, 3]]);
    expect(fuzzy("oi", "order_items")?.ranges).toEqual([[0, 1], [6, 7]]);
    expect(fuzzy("xyz", "orders")).toBeNull();
    expect(fuzzy("ORD", "Orders")).not.toBeNull();
  });

  it("ranks a prefix above a later run, and a run above scattered letters", () => {
    const score = (text: string) => fuzzy("ord", text)!.score;
    expect(score("orders")).toBeGreaterThan(score("purchase_orders"));
    expect(score("purchase_orders")).toBeGreaterThan(score("old_records"));
    expect(score("orders")).toBeGreaterThan(score("orders_archive_2020"));
  });
});
