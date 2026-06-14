import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import type { Entity, Relation, GraphData, EntityDetail, EntityReference, InferenceCandidate, ExtractionCompleteEvent, MergeSuggestion, FeedbackEntry, GraphSettings2D, LintWarning } from "../types/knowledge";
import { DEFAULT_GRAPH_SETTINGS_2D } from "../types/knowledge";
import { getGraphData, getSmartDisplay, getEntityDetail, searchEntities, referenceEntityToChat, getInferences, nexusUpdateEntity, nexusDeleteEntity, nexusAddRelation, nexusUpdateRelation, nexusDeleteRelation, nexusSubmitFeedback, nexusGetPendingMerges, nexusConfirmMerge, nexusIgnoreMerge, nexusBatchOperation, nexusGetEntityFeedback, nexusRunSynthesis, nexusAnalyzeTypes, getConfig, saveConfig } from "../services/api";

const SMART_DISPLAY_THRESHOLD = 500;

interface KnowledgeState {
  entities: Entity[];
  relations: Relation[];
  isLoading: boolean;
  selectedEntityId: string | null;
  selectedEntityDetail: EntityDetail | null;
  navEntityId: string | null;
  setNavEntityId: (id: string | null) => void;
  fetchGraphData: (namespace?: string) => Promise<void>;
  selectEntity: (id: string) => Promise<void>;
  searchQuery: string;
  setSearchQuery: (q: string) => void;
  setupEventListeners: () => Promise<() => void>;

  // Focus mode
  focusedNodeId: string | null;
  focusDepth: number;
  setFocusedNode: (id: string | null) => void;
  setFocusDepth: (depth: number) => void;

  // Orphans
  showOrphans: boolean;
  setShowOrphans: (show: boolean) => void;

  // File nodes
  showFiles: boolean;
  setShowFiles: (show: boolean) => void;

  // Reference to chat
  referenceToChat: (entityId: string) => Promise<EntityReference>;

  // Inference edges
  inferences: InferenceCandidate[];
  showInferenceEdges: boolean;
  loadInferences: (namespace?: string) => Promise<void>;
  setShowInferenceEdges: (show: boolean) => void;

  // ── Nexus P5: Entity editing ──
  updateEntity: (entityId: string, updates: Record<string, unknown>) => Promise<void>;
  deleteEntity: (entityId: string) => Promise<void>;
  addRelation: (fromId: string, toId: string, relationType: string, label?: string, confidence?: number, namespace?: string) => Promise<string>;
  updateRelation: (relationId: string, relationType?: string, label?: string, confidence?: number) => Promise<void>;
  deleteRelation: (relationId: string) => Promise<void>;
  submitFeedback: (entityId: string, action: string, reason?: string) => Promise<void>;
  pendingMerges: MergeSuggestion[];
  loadPendingMerges: () => Promise<void>;
  confirmMerge: (mergeId: string) => Promise<void>;
  ignoreMerge: (mergeId: string) => Promise<void>;
  batchOperation: (action: string, namespace?: string, sourceType?: string, minConfidence?: number, entityIds?: string[]) => Promise<number>;
  entityFeedback: FeedbackEntry[];
  loadEntityFeedback: (entityId: string) => Promise<void>;
  runSynthesis: () => Promise<void>;
  analyzeTypes: () => Promise<void>;

  graphSettings2D: GraphSettings2D;
  settingsOpen: boolean;
  setSettingsOpen: (open: boolean) => void;
  updateGraphSettings2D: (updates: Partial<GraphSettings2D>) => void;
  resetGraphSettings2D: () => void;
  loadGraphSettings: () => Promise<void>;
  saveGraphSettings: () => Promise<void>;

  // 2D-specific panels & state
  showLintPanel: boolean;
  showEntityList: boolean;
  lintWarnings: LintWarning[];
  entityTypeFilter: string | null;
  namespaceFilter: string | null;
  documentViewMode: "entity" | "document";
  animationPlaying: boolean;
  setShowLintPanel: (show: boolean) => void;
  setShowEntityList: (show: boolean) => void;
  fetchLintWarnings: (namespace?: string) => Promise<void>;
  setEntityTypeFilter: (t: string | null) => void;
  setNamespaceFilter: (ns: string | null) => void;
  setDocumentViewMode: (mode: "entity" | "document") => void;
  setAnimationPlaying: (playing: boolean) => void;
}

export const useKnowledgeStore = create<KnowledgeState>((set, get) => ({
  entities: [],
  relations: [],
  isLoading: false,
  selectedEntityId: null,
  selectedEntityDetail: null,
  navEntityId: null,
  setNavEntityId: (id) => set({ navEntityId: id }),
  searchQuery: "",

  focusedNodeId: null,
  focusDepth: 1,

  inferences: [],
  showInferenceEdges: true,

  pendingMerges: [],
  entityFeedback: [],

  fetchGraphData: async (namespace) => {
    set({ isLoading: true });
    try {
      // Use Smart Display for large graphs
      const currentTotal = get().entities.length;
      let data: GraphData;
      if (currentTotal > SMART_DISPLAY_THRESHOLD) {
        const st = get();
        data = await getSmartDisplay(namespace ?? null, {
          mode: "global",
          focal_node: null,
          hops: 2,
          budget: 350,
          min_per_type: 3,
          tier1_cap: 180,
          tier2_cap: 350,
          search_query: st.graphSettings2D.searchQuery || null,
          type_filter: st.entityTypeFilter,
          namespace_filter: st.namespaceFilter,
          min_importance: null,
          show_orphans: st.showOrphans,
        });
      } else {
        data = await getGraphData(namespace);
      }
      set({ entities: data.entities, relations: data.relations, isLoading: false });
    } catch (e) {
      console.error("Failed to fetch graph data:", e);
      set({ isLoading: false });
    }
  },

  selectEntity: async (id) => {
    set({ selectedEntityId: id, selectedEntityDetail: null });
    if (!id) return;
    try {
      const detail: EntityDetail = await getEntityDetail(id);
      set({ selectedEntityDetail: detail });
    } catch (e) {
      console.error("Failed to fetch entity detail:", e);
    }
  },

  setSearchQuery: (q) => set({ searchQuery: q }),

  setFocusedNode: (id) => set({ focusedNodeId: id }),
  setFocusDepth: (depth) => set({ focusDepth: depth }),

  // Orphans
  showOrphans: true,
  setShowOrphans: (show) => set({ showOrphans: show }),

  // File nodes
  showFiles: true,
  setShowFiles: (show) => set({ showFiles: show }),

  referenceToChat: async (entityId) => {
    return referenceEntityToChat(entityId);
  },

  loadInferences: async (_namespace) => {
    try {
      const data = await getInferences(null, 200, "pending");
      set({ inferences: Array.isArray(data) ? data : [] });
    } catch { set({ inferences: [] }); }
  },

  setShowInferenceEdges: (show) => set({ showInferenceEdges: show }),

  // ── Nexus P5: Entity editing ──

  updateEntity: async (entityId, updates) => {
    await nexusUpdateEntity(entityId, updates);
    // Refresh the detail view
    const detail = await getEntityDetail(entityId);
    set({ selectedEntityDetail: detail });
    // Refresh the entity list
    get().fetchGraphData();
  },

  deleteEntity: async (entityId) => {
    await nexusDeleteEntity(entityId);
    set({ selectedEntityId: null, selectedEntityDetail: null });
    get().fetchGraphData();
  },

  addRelation: async (fromId, toId, relationType, label, confidence, namespace) => {
    const id = await nexusAddRelation(fromId, toId, relationType, label, confidence, namespace);
    // Refresh detail if the selected entity is involved
    const state = get();
    if (state.selectedEntityId === fromId || state.selectedEntityId === toId) {
      state.selectEntity(state.selectedEntityId!);
    }
    get().fetchGraphData();
    return id as string;
  },

  updateRelation: async (relationId, relationType, label, confidence) => {
    await nexusUpdateRelation(relationId, relationType, label, confidence);
    const state = get();
    if (state.selectedEntityId) {
      state.selectEntity(state.selectedEntityId);
    }
  },

  deleteRelation: async (relationId) => {
    await nexusDeleteRelation(relationId);
    const state = get();
    if (state.selectedEntityId) {
      state.selectEntity(state.selectedEntityId);
    }
  },

  submitFeedback: async (entityId, action, reason) => {
    await nexusSubmitFeedback(entityId, action, reason);
    // Refresh detail
    const state = get();
    if (state.selectedEntityId === entityId) {
      state.selectEntity(entityId);
    }
    get().fetchGraphData();
  },

  loadPendingMerges: async () => {
    try {
      const data = await nexusGetPendingMerges();
      set({ pendingMerges: Array.isArray(data) ? data : [] });
    } catch (e) {
      console.error("Failed to load pending merges:", e);
    }
  },

  confirmMerge: async (mergeId) => {
    await nexusConfirmMerge(mergeId);
    get().loadPendingMerges();
    get().fetchGraphData();
  },

  ignoreMerge: async (mergeId) => {
    await nexusIgnoreMerge(mergeId);
    get().loadPendingMerges();
  },

  batchOperation: async (action, namespace, sourceType, minConfidence, entityIds) => {
    const result = await nexusBatchOperation(action, namespace, sourceType, minConfidence, entityIds) as { affected: number };
    get().fetchGraphData();
    return result.affected;
  },

  loadEntityFeedback: async (entityId) => {
    try {
      const data = await nexusGetEntityFeedback(entityId);
      set({ entityFeedback: Array.isArray(data) ? data : [] });
    } catch (e) {
      console.error("Failed to load entity feedback:", e);
    }
  },

  runSynthesis: async () => {
    await nexusRunSynthesis();
    get().fetchGraphData();
  },

  analyzeTypes: async () => {
    await nexusAnalyzeTypes();
    get().loadPendingMerges();
  },

  graphSettings2D: { ...DEFAULT_GRAPH_SETTINGS_2D },
  settingsOpen: false,

  setSettingsOpen: (open) => set({ settingsOpen: open }),

  updateGraphSettings2D: (updates) => {
    set((s) => ({
      graphSettings2D: { ...s.graphSettings2D, ...updates },
      showOrphans: updates.showOrphans !== undefined ? updates.showOrphans : s.showOrphans,
      showFiles: updates.showFiles !== undefined ? updates.showFiles : s.showFiles,
    }));
  },

  resetGraphSettings2D: () => set({
    graphSettings2D: { ...DEFAULT_GRAPH_SETTINGS_2D },
    showOrphans: true,
    showFiles: true,
  }),

  loadGraphSettings: async () => {
    try {
      const config = await getConfig() as Record<string, unknown>;
      if (config && config.graph) {
        const g = config.graph as Record<string, unknown>;
        const twoD: Record<string, unknown> = (g.twoD as Record<string, unknown>) || {};
        set({
          graphSettings2D: {
            searchQuery: (twoD.searchQuery as string) ?? DEFAULT_GRAPH_SETTINGS_2D.searchQuery,
            showOrphans: (twoD.showOrphans as boolean) ?? true,
            showFiles: (twoD.showFiles as boolean) ?? true,
            typeFilter: (twoD.typeFilter as string[]) ?? DEFAULT_GRAPH_SETTINGS_2D.typeFilter,
            communityMode: (twoD.communityMode as boolean) ?? DEFAULT_GRAPH_SETTINGS_2D.communityMode,
            inferredCreatable: (twoD.inferredCreatable as boolean) ?? DEFAULT_GRAPH_SETTINGS_2D.inferredCreatable,
            colorGroups: (twoD.colorGroups as GraphSettings2D["colorGroups"]) ?? DEFAULT_GRAPH_SETTINGS_2D.colorGroups,
            typeColors: (twoD.typeColors as Record<string, string>) ?? DEFAULT_GRAPH_SETTINGS_2D.typeColors,
            showArrows: (twoD.showArrows as boolean) ?? DEFAULT_GRAPH_SETTINGS_2D.showArrows,
            showTypeRing: (twoD.showTypeRing as boolean) ?? DEFAULT_GRAPH_SETTINGS_2D.showTypeRing,
            textOpacity: (twoD.textOpacity as number) ?? DEFAULT_GRAPH_SETTINGS_2D.textOpacity,
            edgeOpacity: (twoD.edgeOpacity as number) ?? DEFAULT_GRAPH_SETTINGS_2D.edgeOpacity,
            nodeSize: (twoD.nodeSize as number) ?? DEFAULT_GRAPH_SETTINGS_2D.nodeSize,
            linkThickness: (twoD.linkThickness as number) ?? DEFAULT_GRAPH_SETTINGS_2D.linkThickness,
            centerForce: (twoD.centerForce as number) ?? DEFAULT_GRAPH_SETTINGS_2D.centerForce,
            repelForce: (twoD.repelForce as number) ?? DEFAULT_GRAPH_SETTINGS_2D.repelForce,
            attractForce: (twoD.attractForce as number) ?? DEFAULT_GRAPH_SETTINGS_2D.attractForce,
            linkLength: (twoD.linkLength as number) ?? DEFAULT_GRAPH_SETTINGS_2D.linkLength,
            dragForce: (twoD.dragForce as number) ?? DEFAULT_GRAPH_SETTINGS_2D.dragForce,
          },
          showOrphans: (twoD.showOrphans as boolean) ?? true,
          showFiles: (twoD.showFiles as boolean) ?? true,
        });
        try {
          const etf = localStorage.getItem("hermes_knowledge_type_filter");
          const nsf = localStorage.getItem("hermes_knowledge_namespace_filter");
          if (etf) set({ entityTypeFilter: etf === "__all__" ? null : etf });
          if (nsf) set({ namespaceFilter: nsf === "__all__" ? null : nsf });
        } catch {}
      }
    } catch {}
  },

  saveGraphSettings: async () => {
    try {
      const { graphSettings2D } = get();
      await saveConfig({ graph: { twoD: graphSettings2D } });
      try {
        const state = get();
        if (state.entityTypeFilter) localStorage.setItem("hermes_knowledge_type_filter", state.entityTypeFilter);
        else localStorage.setItem("hermes_knowledge_type_filter", "__all__");
        if (state.namespaceFilter) localStorage.setItem("hermes_knowledge_namespace_filter", state.namespaceFilter);
        else localStorage.setItem("hermes_knowledge_namespace_filter", "__all__");
      } catch {}
    } catch (e) {
      console.error("Failed to save graph settings:", e);
    }
  },

  // ── 2D panels & state ──
  showLintPanel: false,
  showEntityList: false,
  lintWarnings: [],
  entityTypeFilter: null,
  namespaceFilter: null,
  documentViewMode: "entity",
  animationPlaying: false,

  setShowLintPanel: (show) => set({ showLintPanel: show }),

  setShowEntityList: (show) => set({ showEntityList: show }),

  fetchLintWarnings: async (namespace) => {
    try {
      const { getLintWarnings } = await import("../services/api");
      const data = await getLintWarnings(namespace ?? null);
      set({ lintWarnings: Array.isArray(data) ? data : [] });
    } catch (e) {
      console.error("Failed to fetch lint warnings:", e);
    }
  },

  setEntityTypeFilter: (t) => {
    set({ entityTypeFilter: t });
    try { localStorage.setItem("hermes_knowledge_type_filter", t || "__all__"); } catch {}
  },

  setNamespaceFilter: (ns) => {
    set({ namespaceFilter: ns });
    try { localStorage.setItem("hermes_knowledge_namespace_filter", ns || "__all__"); } catch {}
  },

  setDocumentViewMode: (mode) => set({ documentViewMode: mode }),

  setAnimationPlaying: (playing) => set({ animationPlaying: playing }),

  setupEventListeners: async () => {
    const u1 = await listen<ExtractionCompleteEvent>(
      "knowledge:extraction-complete",
      () => {
        get().fetchGraphData();
      },
    );
    const u2 = await listen("knowledge:graph-updated", () => {
      get().fetchGraphData();
    });
    return () => {
      u1();
      u2();
    };
  },
}));
