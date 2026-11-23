import { describe, expect, it } from "vitest";
import { describeCommand, detectMode, formatBytes, formatTtl, parseTtl, prefixPattern, prefixTree, splitKey, typeBadge } from "./kv";

describe("time to live", () => {
  it("shows the two largest units", () => {
    expect(formatTtl(41_000)).toBe("41s");
    expect(formatTtl((23 * 60 + 14) * 1000)).toBe("23m 14s");
    expect(formatTtl((3 * 86400 + 4 * 3600 + 59) * 1000)).toBe("3d 4h");
    expect(formatTtl(3600_000)).toBe("1h");
    expect(formatTtl(300)).toBe("0s");
  });

  it("reads seconds and unit forms", () => {
    expect(parseTtl("90")).toBe(90);
    expect(parseTtl("15m")).toBe(900);
    expect(parseTtl("2h 30m")).toBe(9000);
    expect(parseTtl("3D")).toBe(259200);
    expect(parseTtl("0")).toBeNull();
    expect(parseTtl("soon")).toBeNull();
    expect(parseTtl("-5")).toBeNull();
    expect(parseTtl("")).toBeNull();
  });
});

describe("key names and sizes", () => {
  it("splits a key at its last colon", () => {
    expect(splitKey("cart:48213")).toEqual(["cart:", "48213"]);
    expect(splitKey("a:b:c")).toEqual(["a:b:", "c"]);
    expect(splitKey("plain")).toEqual(["", "plain"]);
    expect(splitKey("odd:")).toEqual(["", "odd:"]);
  });

  it("formats sizes and type badges", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(64277709)).toBe("61.3 MB");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(typeBadge("string")).toBe("STR");
    expect(typeBadge("zset")).toBe("ZSET");
    expect(typeBadge("ReJSON-RL")).toBe("REJSO");
  });
});

describe("commands and values", () => {
  it("writes the command a change runs", () => {
    expect(describeCommand({ op: "set_string", key: "k", value: "two words" })).toBe('SET k "two words" KEEPTTL');
    expect(describeCommand({ op: "list_push", key: "q", value: "x", left: true })).toBe("LPUSH q x");
    expect(describeCommand({ op: "zset_add", key: "z", member: "ada", score: 1.5 })).toBe("ZADD z 1.5 ada");
    expect(describeCommand({ op: "delete", key: "my key" })).toBe('DEL "my key"');
  });

  it("detects how to show a string", () => {
    expect(detectMode({ type: "text", value: '{"a":1}' })).toBe("json");
    expect(detectMode({ type: "text", value: "{not json" })).toBe("text");
    expect(detectMode({ type: "text", value: "42" })).toBe("text");
    expect(detectMode({ type: "bytes", value: "AAE=" })).toBe("hex");
  });
});

describe("key prefixes", () => {
  it("groups keys on their colons, level by level", () => {
    const tree = prefixTree(["user:1", "user:2:cart", "user:2:name", "session:x", "plain", "odd:", "user:2:a:b:c"], 2);
    expect(tree.map((g) => [g.prefix, g.label, g.count])).toEqual([
      ["session:", "session", 1],
      ["user:", "user", 4],
    ]);
    expect(tree[1].children.map((g) => [g.prefix, g.label, g.count])).toEqual([["user:2:", "2", 3]]);
    // The depth stops the split.
    expect(tree[1].children[0].children).toEqual([]);
  });

  it("escapes glob characters in the pattern", () => {
    expect(prefixPattern("user:")).toBe("user:*");
    expect(prefixPattern("a*b?[c]:")).toBe("a\\*b\\?\\[c\\]:*");
  });
});
