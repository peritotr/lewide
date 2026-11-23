// Small pieces of the key-value view that are worth testing on their own.
import type { KeyCommand, Value } from "./api";

/** `23m 14s`, `41s`, `3d 4h`: the two largest units that are not zero. */
export function formatTtl(ms: number): string {
  let seconds = Math.max(0, Math.round(ms / 1000));
  const parts: string[] = [];
  for (const [size, unit] of [[86400, "d"], [3600, "h"], [60, "m"], [1, "s"]] as const) {
    const n = Math.floor(seconds / size);
    seconds -= n * size;
    if (n > 0 || (unit === "s" && parts.length === 0)) parts.push(`${n}${unit}`);
    else if (parts.length > 0) break;
    if (parts.length === 2) break;
  }
  return parts.join(" ");
}

/** Reads `90`, `41s`, `15m`, `2h 30m` or `3d` as seconds; null when it is not a duration. */
export function parseTtl(text: string): number | null {
  const trimmed = text.trim().toLowerCase();
  if (/^\d+$/.test(trimmed)) return Number(trimmed) > 0 ? Number(trimmed) : null;
  if (!/^(\d+\s*[dhms]\s*)+$/.test(trimmed)) return null;
  let total = 0;
  for (const [, n, unit] of trimmed.matchAll(/(\d+)\s*([dhms])/g)) total += Number(n) * { d: 86400, h: 3600, m: 60, s: 1 }[unit as "d" | "h" | "m" | "s"];
  return total > 0 && Number.isSafeInteger(total) ? total : null;
}

/** The part of a key up to its last `:`, drawn muted, and the rest. */
export function splitKey(key: string): [prefix: string, name: string] {
  const at = key.lastIndexOf(":");
  return at < 0 || at === key.length - 1 ? ["", key] : [key.slice(0, at + 1), key.slice(at + 1)];
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  for (const [size, unit] of [[1024 ** 3, "GB"], [1024 ** 2, "MB"], [1024, "KB"]] as const) {
    if (bytes >= size) return `${(bytes / size).toFixed(bytes >= size * 100 ? 0 : 1)} ${unit}`;
  }
  return `${bytes} B`;
}

/** The store's type name as the badge shows it. */
export function typeBadge(kind: string): string {
  return { string: "STR", hash: "HASH", list: "LIST", set: "SET", zset: "ZSET", stream: "STRM" }[kind] ?? kind.slice(0, 5).toUpperCase();
}

const word = (text: string) => (/^[^\s"']+$/.test(text) ? text : JSON.stringify(text));
const short = (text: string) => word(text.length > 60 ? `${text.slice(0, 60)}…` : text);

/** The command a change will run, for the question asked before a write on Production. */
export function describeCommand(command: KeyCommand): string {
  const key = word(command.key);
  switch (command.op) {
    case "set_string":
      return `SET ${key} ${short(command.value)} KEEPTTL`;
    case "hash_set":
      return `HSET ${key} ${word(command.field)} ${short(command.value)}`;
    case "hash_delete":
      return `HDEL ${key} ${word(command.field)}`;
    case "list_push":
      return `${command.left ? "LPUSH" : "RPUSH"} ${key} ${short(command.value)}`;
    case "list_set":
      return `LSET ${key} ${command.index} ${short(command.value)}`;
    case "list_remove":
      return `LREM ${key} (index ${command.index})`;
    case "set_add":
      return `SADD ${key} ${short(command.member)}`;
    case "set_remove":
      return `SREM ${key} ${short(command.member)}`;
    case "zset_add":
      return `ZADD ${key} ${command.score} ${short(command.member)}`;
    case "zset_remove":
      return `ZREM ${key} ${short(command.member)}`;
    case "delete":
      return `DEL ${key}`;
    case "rename":
      return `RENAMENX ${key} ${word(command.to)}`;
    case "expire":
      return `EXPIRE ${key} ${command.seconds}`;
    case "persist":
      return `PERSIST ${key}`;
  }
}

/** A stored value as text to edit, or null when it is binary and cannot be typed. */
export function valueText(value: Value): string | null {
  if (value.type === "bytes") return null;
  if (value.type === "null") return "";
  return value.type === "json" ? JSON.stringify(value.value) : String(value.value);
}

/** Which of the string editor's modes fits a value. */
export function detectMode(value: Value): "text" | "json" | "hex" {
  const text = valueText(value);
  if (text === null) return "hex";
  const trimmed = text.trim();
  if (!/^[[{]/.test(trimmed)) return "text";
  try {
    JSON.parse(trimmed);
    return "json";
  } catch {
    return "text";
  }
}

/** Why a text is not JSON, or null when it is. */
export function jsonError(text: string): string | null {
  try {
    JSON.parse(text);
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
}
