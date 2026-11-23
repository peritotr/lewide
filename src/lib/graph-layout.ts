// A small force-directed layout for the graph view. It runs to the end in one
// call, so the canvas settles at once and nothing keeps moving afterwards.

export interface Pos {
  x: number;
  y: number;
}

const SPACING = 110;

/**
 * Places `nodes` so that linked ones sit near each other and none overlap.
 * Nodes in `fixed` stay where they are (the ones already on the canvas when
 * neighbours are added); new ones start at `near` or on a circle.
 */
export function layout(nodes: string[], edges: [string, string][], fixed: Record<string, Pos> = {}, near: Pos | null = null, iterations = 300): Record<string, Pos> {
  const pos: Record<string, Pos> = {};
  const free = nodes.filter((id) => !(id in fixed));
  for (const id of nodes) if (id in fixed) pos[id] = { ...fixed[id] };
  const radius = Math.max(SPACING, (SPACING * Math.sqrt(nodes.length)) / 2);
  free.forEach((id, i) => {
    // The golden angle spreads starting points evenly and the same way every time.
    const angle = i * 2.399963;
    const r = near ? 40 + 12 * Math.sqrt(i) : radius * Math.sqrt((i + 0.5) / free.length);
    pos[id] = { x: (near?.x ?? 0) + Math.cos(angle) * r, y: (near?.y ?? 0) + Math.sin(angle) * r };
  });
  if (free.length === 0) return pos;

  const known = new Set(nodes);
  const links = edges.filter(([a, b]) => a !== b && known.has(a) && known.has(b));
  for (let step = 0; step < iterations; step++) {
    const heat = SPACING * 0.5 * (1 - step / iterations) + 0.5;
    const move: Record<string, Pos> = Object.fromEntries(free.map((id) => [id, { x: 0, y: 0 }]));
    for (const a of free) {
      for (const b of nodes) {
        if (a === b) continue;
        let dx = pos[a].x - pos[b].x;
        let dy = pos[a].y - pos[b].y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 0.01) {
          // Two nodes on the same spot are pushed apart in a direction that depends on their order.
          dx = a < b ? 0.1 : -0.1;
          dy = 0.1;
          d2 = 0.02;
        }
        const force = (SPACING * SPACING) / d2;
        move[a].x += dx * force;
        move[a].y += dy * force;
      }
      // A weak pull to the middle keeps unlinked nodes from drifting away.
      move[a].x -= (pos[a].x - (near?.x ?? 0)) * 0.02;
      move[a].y -= (pos[a].y - (near?.y ?? 0)) * 0.02;
    }
    for (const [a, b] of links) {
      const dx = pos[a].x - pos[b].x;
      const dy = pos[a].y - pos[b].y;
      const d = Math.sqrt(dx * dx + dy * dy) || 0.1;
      const force = d / SPACING;
      if (a in move) (move[a].x -= dx * force), (move[a].y -= dy * force);
      if (b in move) (move[b].x += dx * force), (move[b].y += dy * force);
    }
    for (const id of free) {
      const length = Math.sqrt(move[id].x ** 2 + move[id].y ** 2) || 1;
      const by = Math.min(length, heat);
      pos[id].x += (move[id].x / length) * by;
      pos[id].y += (move[id].y / length) * by;
    }
  }
  return pos;
}

/** The pan and zoom that show every position inside a box, with room around the edge. */
export function fit(positions: Pos[], width: number, height: number, margin = 60): { x: number; y: number; k: number } {
  if (positions.length === 0 || width <= 0 || height <= 0) return { x: width / 2, y: height / 2, k: 1 };
  const xs = positions.map((p) => p.x);
  const ys = positions.map((p) => p.y);
  const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const k = Math.max(0.1, Math.min(1.5, (width - margin * 2) / Math.max(1, maxX - minX), (height - margin * 2) / Math.max(1, maxY - minY)));
  return { x: width / 2 - ((minX + maxX) / 2) * k, y: height / 2 - ((minY + maxY) / 2) * k, k };
}

/**
 * One letter per label, all different: colours repeat after four labels, so
 * the letter is what tells them apart. A label takes its first letter, or the
 * next letter of its name that is still free, or a digit.
 */
export function labelLetters(labels: string[]): Record<string, string> {
  const taken = new Set<string>();
  const out: Record<string, string> = {};
  for (const label of labels) {
    const candidates = [...label.toUpperCase()].filter((c) => /\p{L}|\p{N}/u.test(c));
    const letter = candidates.find((c) => !taken.has(c)) ?? [..."123456789"].find((c) => !taken.has(c)) ?? "·";
    taken.add(letter);
    out[label] = label === "" ? "·" : letter;
  }
  return out;
}

/** What a node is called on the canvas: the first of name, title, id among its properties. */
export function caption(properties: Record<string, unknown>): string {
  for (const key of ["name", "title", "id"]) {
    const value = properties[key];
    if (typeof value === "string" || typeof value === "number") return String(value);
  }
  return "";
}
