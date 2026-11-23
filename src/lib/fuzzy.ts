// Matching for the command palette: the typed letters, in order, anywhere in the name.

export interface Match {
  /** Higher is better. */
  score: number;
  /** Start and end offsets of the matched pieces, for highlighting. */
  ranges: [number, number][];
}

/** Null when `query` is not a subsequence of `text`. Case is ignored. */
export function fuzzy(query: string, text: string): Match | null {
  const q = query.toLowerCase();
  const s = text.toLowerCase();
  if (q === "") return { score: 0, ranges: [] };
  // A run of the query as it stands beats scattered letters.
  const at = s.indexOf(q);
  if (at >= 0) {
    const boundary = at === 0 || /[^a-z0-9]/.test(s[at - 1]);
    return { score: 1000 - at - (boundary ? 0 : 50) - (s.length - q.length), ranges: [[at, at + q.length]] };
  }
  const ranges: [number, number][] = [];
  let from = 0;
  let gaps = 0;
  for (const letter of q) {
    const found = s.indexOf(letter, from);
    if (found < 0) return null;
    const last = ranges[ranges.length - 1];
    if (last && last[1] === found) last[1] = found + 1;
    else {
      ranges.push([found, found + 1]);
      gaps += found - from;
    }
    from = found + 1;
  }
  return { score: 400 - gaps * 4 - ranges.length * 20 - s.length, ranges };
}
