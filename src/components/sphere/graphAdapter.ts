import type { Entity, Relation, InferenceCandidate } from "../../types/knowledge";

// Vibrant type-based colors — one per major entity category.
// Only inferred entities get gray (#888888).
const TYPE_COLORS: Record<string, string> = {
  __file__:        "#e8e8e8",  // 文档: 浅灰白
  document:        "#e8e8e8",
  location:        "#4CAF50",  // 地名: 绿色
  organization:    "#FF9800",  // 组织: 橙色
  person:          "#E91E63",  // 人物: 粉色
  natural_feature: "#8BC34A",  // 自然景观: 浅绿
  time:            "#00BCD4",  // 时间: 青色
  concept:         "#7C4DFF",  // 概念: 紫色
  project:         "#FF5722",  // 项目: 深橙
  tool:            "#2196F3",  // 工具: 蓝色
  inferred:        "#888888",  // 推断: 灰色(唯一!)
};

const TYPE_COLORS_LIST = Object.values(TYPE_COLORS);
let _tcIdx = 0;
function typeColor(et: string): string {
  if (TYPE_COLORS[et]) return TYPE_COLORS[et];
  // Unknown types cycle through the palette
  return TYPE_COLORS_LIST[_tcIdx++ % TYPE_COLORS_LIST.length];
}

export interface FGNode {
  id: string;
  name: string;
  entityType: string;
  degree: number;
  val: number;
  color: string;
  isOrphan: boolean;
  isInferred: boolean;
  _entity: Entity;
  _sphereRadius: number;
}

export interface FGLink {
  source: string;
  target: string;
  weight: number;
  merged: boolean;
  relationTypes: string[];
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
    minDegree?: number;
    minImportance?: number;
    typeFilter?: string[];
    colorGroups?: ColorGroup[];
    typeColors?: Record<string, string>;
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
  const minDeg = opts.minDegree ?? 0;
  const minImp = opts.minImportance ?? 0;
  const typeFilter = (opts.typeFilter || []).map(t => t.toLowerCase());
  const groups = opts.colorGroups ?? [];

  const visibleIds = new Set<string>();
  const nodes: FGNode[] = [];

  for (const e of entities) {
    if (e.hidden) continue;
    if (!opts.showFiles && e.entity_type === "__file__") continue;
    const degree = degMap.get(e.id) ?? 0;
    if (!opts.showOrphans && degree === 0) continue;
    if (degree < minDeg) continue;
    if (typeFilter.length > 0 && !typeFilter.includes(e.entity_type.toLowerCase())) continue;
    if (opts.focusedNodeId && e.id !== opts.focusedNodeId && !focusNeighbors.has(e.id)) continue;
    if (searchLower && !e.name.toLowerCase().includes(searchLower)) continue;
    if (minImp > 0 && (e.importance ?? 0) < minImp) continue;

    visibleIds.add(e.id);
    const isOrphan = degree === 0;
    const isInferred = e.inferred === true;
    const isFile = e.entity_type === "__file__" || e.entity_type === "document";

    // Color logic:
    //   - Inferred entities ONLY → gray (#888888, locked)
    //   - File/Document nodes → light gray-white
    //   - User custom typeColors → highest priority for non-inferred
    //   - Color group regex match → next
    //   - Built-in type-based color → fallback
    let color: string;
    if (isInferred) {
      color = "#888888"; // 灰色 — 仅推断实体，不可覆盖
    } else if (isFile) {
      color = opts.typeColors?.["__file__"] ?? "#e8e8e8";
    } else {
      const custom = opts.typeColors?.[e.entity_type];
      const cgColor = matchColorGroup(e.name, groups);
      color = custom ?? cgColor ?? typeColor(e.entity_type);
    }

    nodes.push({
      id: e.id,
      name: e.name,
      entityType: e.entity_type,
      degree,
      val: nodeVal(degree),
      color,
      isOrphan,
      isInferred,
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
      });
    }
  }

  return { nodes, links, infLinks };
}
