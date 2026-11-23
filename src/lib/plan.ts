// Reads PostgreSQL's EXPLAIN (FORMAT JSON) output into rows for the plan view.

export interface PlanNode {
  depth: number;
  /** Whether this is the last child of its parent, for drawing the connector. */
  last: boolean;
  type: string;
  /** What the node works on: relation, index, condition. */
  detail: string;
  totalCost: number;
  /** This node's cost without its children's. */
  ownCost: number;
  rows: number;
  /** Own cost as a fraction of the plan's total, 0 to 1. */
  share: number;
  mostExpensive: boolean;
}

export interface Plan {
  nodes: PlanNode[];
  totalCost: number;
}

type RawNode = Record<string, unknown> & { Plans?: RawNode[] };

const number = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : 0);

function detailOf(node: RawNode): string {
  const parts: string[] = [];
  if (typeof node["Relation Name"] === "string") {
    const alias = node.Alias && node.Alias !== node["Relation Name"] ? ` ${String(node.Alias)}` : "";
    parts.push(`on ${node["Relation Name"]}${alias}`);
  }
  if (typeof node["Index Name"] === "string") parts.push(`using ${node["Index Name"]}`);
  for (const key of ["Hash Cond", "Merge Cond", "Join Filter", "Index Cond", "Recheck Cond", "Filter"]) {
    if (typeof node[key] === "string") parts.push(`${key === "Filter" ? "filter" : "cond"} ${node[key]}`);
  }
  if (Array.isArray(node["Sort Key"])) parts.push(`by ${node["Sort Key"].join(", ")}`);
  if (Array.isArray(node["Group Key"])) parts.push(`group ${node["Group Key"].join(", ")}`);
  return parts.join(" · ");
}

/** The plan as a flat list in display order, or null when `json` is not an EXPLAIN result. */
export function parsePlan(json: unknown): Plan | null {
  const root = (Array.isArray(json) ? json[0] : json) as { Plan?: RawNode } | null;
  if (!root || typeof root !== "object" || !root.Plan) return null;

  const nodes: PlanNode[] = [];
  const totalCost = number(root.Plan["Total Cost"]);
  const walk = (node: RawNode, depth: number, last: boolean) => {
    const children = Array.isArray(node.Plans) ? node.Plans : [];
    const total = number(node["Total Cost"]);
    // A node's total includes its inputs; what is left is the work it does itself.
    const own = Math.max(0, total - children.reduce((sum, child) => sum + number(child["Total Cost"]), 0));
    const strategy = typeof node.Strategy === "string" && node["Node Type"] === "Aggregate" ? `${node.Strategy} ` : "";
    nodes.push({
      depth,
      last,
      type: `${strategy}${String(node["Node Type"] ?? "Node")}`,
      detail: detailOf(node),
      totalCost: total,
      ownCost: own,
      rows: number(node["Plan Rows"]),
      share: totalCost > 0 ? own / totalCost : 0,
      mostExpensive: false,
    });
    children.forEach((child, i) => walk(child, depth + 1, i === children.length - 1));
  };
  walk(root.Plan, 0, true);

  const top = nodes.reduce((best, node) => (node.ownCost > best.ownCost ? node : best), nodes[0]);
  if (top.ownCost > 0) top.mostExpensive = true;
  return { nodes, totalCost };
}

/** One plain sentence about the plan, or null when there is nothing worth saying. */
export function readPlan(plan: Plan): string | null {
  const top = plan.nodes.find((n) => n.mostExpensive);
  if (!top || top.share < 0.3) return null;
  const share = `${Math.round(top.share * 100)}%`;
  if (top.type === "Seq Scan") {
    return `Most of the cost (${share}) is a full scan ${top.detail.split(" · ")[0]}: every row is read. An index on the filtered or joined column may avoid it.`;
  }
  return `Most of the cost (${share}) is in ${top.type}${top.detail ? ` ${top.detail.split(" · ")[0]}` : ""}.`;
}
