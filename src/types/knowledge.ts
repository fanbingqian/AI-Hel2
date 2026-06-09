export interface Entity {
  id: string;
  name: string;
  entity_type: string;
  description: string;
  aliases: string[];
  properties: Record<string, unknown>;
  confidence: number;
  importance?: number;
  importance_level?: string;
  source_file?: string;
  namespace?: string;
  created_at: string;
  updated_at: string;
  color?: string;
  hidden: boolean;
  display_tier?: number;
}

export interface Relation {
  id: string;
  from_id: string;
  to_id: string;
  relation_type: string;
  label?: string;
  weight: number;
  bidirectional: boolean;
}

export interface GraphData {
  entities: Entity[];
  relations: Relation[];
  namespace?: string;
  total_entity_count: number;
  offline: boolean;
}

export interface EntityDetail {
  entity: Entity;
  inbound_relations: Relation[];
  outbound_relations: Relation[];
  lint_warnings: Array<{
    warning_type: string;
    entity_id?: string;
    entity_name: string;
    message: string;
    severity: string;
  }>;
}

export interface EntitySummary {
  id: string;
  name: string;
  entity_type: string;
  description: string;
  match_score: number;
}

export interface ExtractionCompleteEvent {
  new_count: number;
  updated_count: number;
  source_file: string;
  snapshot_updated: boolean;
}

export function getTypeColor(type: string): string {
  let hash = 0;
  for (let i = 0; i < type.length; i++) {
    hash = type.charCodeAt(i) + ((hash << 5) - hash);
  }
  const hue = Math.abs(hash) % 360;
  return `hsl(${hue}, 60%, 55%)`;
}

export interface EntityReference {
  entity_name: string;
  entity_type: string;
  summary: string;
  markdown_ref: string;
}

export interface InferenceCandidate {
  id: string;
  from_id: string;
  to_id: string;
  relation_type: string;
  confidence: number;
  reason: string;
  status: string;
}

export interface SmartDisplayConfig {
  mode: string;
  focal_node: string | null;
  hops: number;
  budget: number;
  min_per_type: number;
  tier1_cap: number;
  tier2_cap: number;
}

export interface SphereNode {
  id: string;
  entityId: string;
  name: string;
  entity_type: string;
  x: number;
  y: number;
  z: number;
  size: number;
}

// ── Nexus P5: Entity editor types ──

export interface MergeSuggestion {
  id: string;
  category: string;
  type_name: string;
  usage_count: number;
  canonical_suggestion: string;
  similar_types: string;
  status: string;
  last_analyzed: string | null;
}

export interface FeedbackEntry {
  id: string;
  entity_id: string;
  entity_name: string;
  source_type: string;
  action: string;
  reason: string;
  created_at: string;
}

export interface BatchOperationResult {
  affected: number;
  action: string;
}

export interface EntityUpdates {
  name?: string;
  entity_type?: string;
  namespace?: string;
  description?: string;
  confidence?: number;
  hidden?: boolean;
}

export interface GraphSettings3D {
  repulsion: number;
  attraction: number;
  linkDistance: number;
  centering: number;
  showLabels: boolean;
  showInferenceEdges: boolean;
  nodeScale: number;
  edgeOpacity: number;
}

/** @deprecated use GraphSettings3D */
export type GraphSettings = GraphSettings3D;

export const DEFAULT_GRAPH_SETTINGS_3D: GraphSettings3D = {
  repulsion: 1200,
  attraction: 0.005,
  linkDistance: 80,
  centering: 0.006,
  showLabels: true,
  showInferenceEdges: true,
  nodeScale: 1.0,
  edgeOpacity: 0.9,
};

/** @deprecated use DEFAULT_GRAPH_SETTINGS_3D */
export const DEFAULT_GRAPH_SETTINGS = DEFAULT_GRAPH_SETTINGS_3D;

export interface GraphSettings2D {
  // 筛选
  searchQuery: string;
  showTags: boolean;
  showAttachments: boolean;
  showOrphans: boolean;
  minImportance: number;
  explorationDepth: number;
  colorGroups: Array<{ id: string; name: string; color: string; pattern: string }>;
  // 外观
  showArrows: boolean;
  showTypeRing: boolean;
  textOpacity: number;
  nodeSize: number;
  linkThickness: number;
  // 力度
  centerForce: number;
  repelForce: number;
  attractForce: number;
  linkLength: number;
  dragForce: number;
}

export const DEFAULT_GRAPH_SETTINGS_2D: GraphSettings2D = {
  searchQuery: "",
  showTags: true,
  showAttachments: true,
  showOrphans: true,
  minImportance: 0,
  explorationDepth: 2,
  colorGroups: [],
  showArrows: false,
  showTypeRing: true,
  textOpacity: 0.85,
  nodeSize: 1.5,
  linkThickness: 1.5,
  centerForce: 0.2,
  repelForce: 10,
  attractForce: 0.5,
  linkLength: 160,
  dragForce: 3,
};

export interface LintWarning {
  warning_type: string;
  entity_id?: string;
  entity_name: string;
  message: string;
  severity: string;
}

export const RELATION_LABELS_ZH: Record<string, string> = {
  related_to: "相关",
  depends_on: "依赖",
  contains: "包含",
  same_as: "等同",
  created_by: "创建者",
  uses: "使用",
  opposes: "对立",
  part_of: "组成部分",
};
