import type { Entity, Relation, InferenceCandidate } from "../../types/knowledge";

export interface FGNode {
  id: string;
  name: string;
  entityType: string;
  degree: number;
  val: number;
  color: string;
  isFile: boolean;
  isOrphan: boolean;
  _entity: Entity;
  _sphereRadius: number;
}

export interface FGLink {
  source: string;
  target: string;
  weight: number;
  merged: boolean;
  relationTypes: string[];
  isFileEdge: boolean;
  _relation?: Relation;
}

interface ColorGroup {
  id: string;
  name: string;
  color: string;
  pattern: string;
}

function matchColorGroup(name: string, groups: ColorGroup[]): string | null {
  for (const g of groups) {
    if (!g.pattern.trim()) continue;
    try {
      if (new RegExp(g.pattern, "i").test(name)) return g.color;
    } catch { /* skip */ }
  }
  return null;
}

function nodeVal(degree: number): number {
  return 1 + Math.min(degree, 12) * 0.4;
}

function nodeRadius(degree: number): number {
  return (4 + Math.min(degree, 12) * 0.5);
}

export function buildGraphData(
  entities: Entity[],
  relations: Relation[],
  inferences: InferenceCandidate[],
  opts: {
    focusedNodeId: string | null;
    focusDepth: number;
    showOrphans: boolean;
    showFiles: boolean;
    showInferenceEdges: boolean;
    nodeRelSize: number;
    searchQuery?: string;
    minImportance?: number;
    colorGroups?: ColorGroup[];
  },
) {
  const degMap = new Map<string, number>();
  for (const r of relations) {
    degMap.set(r.from_id, (degMap.get(r.from_id) ?? 0) + 1);
    degMap.set(r.to_id, (degMap.get(r.to_id) ?? 0) + 1);
  }

  let focusNeighbors = new Set<string>();
  if (opts.focusedNodeId) {
    const adj = new Map<string, string[]>();
    for (const r of relations) {
      if (!adj.has(r.from_id)) adj.set(r.from_id, []);
      if (!adj.has(r.to_id)) adj.set(r.to_id, []);
      adj.get(r.from_id)!.push(r.to_id);
      if (r.bidirectional) adj.get(r.to_id)!.push(r.from_id);
    }
    const visited = new Set<string>([opts.focusedNodeId]);
    let frontier = [opts.focusedNodeId];
    for (let d = 0; d < opts.focusDepth; d++) {
      const next: string[] = [];
      for (const nid of frontier) {
        for (const nb of adj.get(nid) ?? []) {
          if (!visited.has(nb)) {
            visited.add(nb);
            focusNeighbors.add(nb);
            next.push(nb);
          }
        }
      }
      frontier = next;
      if (frontier.length === 0) break;
    }
  }

  const searchLower = (opts.searchQuery || "").toLowerCase().trim();
  const minImp = opts.minImportance ?? 0;
  const groups = opts.colorGroups ?? [];

  const visibleIds = new Set<string>();
  const nodes: FGNode[] = [];

  for (const e of entities) {
    if (e.hidden) continue;
    if (!opts.showFiles && e.entity_type === "__file__") continue;
    const degree = degMap.get(e.id) ?? 0;
    if (!opts.showOrphans && degree === 0) continue;
    if (opts.focusedNodeId && e.id !== opts.focusedNodeId && !focusNeighbors.has(e.id)) continue;
    if (searchLower && !e.name.toLowerCase().includes(searchLower)) continue;
    if (minImp > 0 && (e.importance ?? 0) < minImp) continue;

    visibleIds.add(e.id);
    const isFile = e.entity_type === "__file__";
    const isOrphan = degree === 0;
    const cgColor = matchColorGroup(e.name, groups);
    const color = cgColor ?? "#8b95a3"; // Obsidian --graph-node default gray

    nodes.push({
      id: e.id,
      name: e.name,
      entityType: e.entity_type,
      degree,
      val: nodeVal(degree),
      color,
      isFile,
      isOrphan,
      _entity: e,
      _sphereRadius: nodeRadius(degree),
    });
  }

  const edgeGroups = new Map<string, { source: string; target: string; weight: number; relationTypes: string[] }>();
  for (const r of relations) {
    if (!visibleIds.has(r.from_id) || !visibleIds.has(r.to_id)) continue;
    const key = [r.from_id, r.to_id].sort().join("||");
    const existing = edgeGroups.get(key);
    if (existing) {
      existing.weight = Math.max(existing.weight, r.weight);
      if (!existing.relationTypes.includes(r.relation_type)) {
        existing.relationTypes.push(r.relation_type);
      }
    } else {
      edgeGroups.set(key, { source: r.from_id, target: r.to_id, weight: r.weight, relationTypes: [r.relation_type] });
    }
  }

  const links: FGLink[] = Array.from(edgeGroups.values()).map((g) => ({
    source: g.source,
    target: g.target,
    weight: g.weight,
    merged: g.relationTypes.length > 1,
    relationTypes: g.relationTypes,
    isFileEdge: g.relationTypes.some((rt) => rt === "contains" || rt === "wikilink"),
  }));

  const infLinks: FGLink[] = [];
  if (opts.showInferenceEdges) {
    for (const inf of inferences) {
      if (!visibleIds.has(inf.from_id) || !visibleIds.has(inf.to_id)) continue;
      infLinks.push({
        source: inf.from_id,
        target: inf.to_id,
        weight: inf.confidence,
        merged: false,
        relationTypes: ["__inference__"],
        isFileEdge: false,
      });
    }
  }

  return { nodes, links, infLinks };
}
