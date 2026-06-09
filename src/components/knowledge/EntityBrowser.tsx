import { useMemo, useState, useEffect, useCallback } from "react";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { useUIStore } from "../../stores/uiStore";
import { getTypeColor } from "../../types/knowledge";
import type { MergeSuggestion } from "../../types/knowledge";
import styles from "./EntityBrowser.module.css";

type DetailMode = "view" | "edit";

export function EntityBrowser() {
  const entities = useKnowledgeStore((s) => s.entities);
  const relations = useKnowledgeStore((s) => s.relations);
  const selectEntity = useKnowledgeStore((s) => s.selectEntity);
  const selectedEntityId = useKnowledgeStore((s) => s.selectedEntityId);
  const selectedEntityDetail = useKnowledgeStore((s) => s.selectedEntityDetail);
  const updateEntity = useKnowledgeStore((s) => s.updateEntity);
  const deleteEntity = useKnowledgeStore((s) => s.deleteEntity);
  const addRelation = useKnowledgeStore((s) => s.addRelation);
  const updateRelation = useKnowledgeStore((s) => s.updateRelation);
  const deleteRelation = useKnowledgeStore((s) => s.deleteRelation);
  const submitFeedback = useKnowledgeStore((s) => s.submitFeedback);
  const pendingMerges = useKnowledgeStore((s) => s.pendingMerges);
  const loadPendingMerges = useKnowledgeStore((s) => s.loadPendingMerges);
  const confirmMerge = useKnowledgeStore((s) => s.confirmMerge);
  const ignoreMerge = useKnowledgeStore((s) => s.ignoreMerge);
  const batchOperation = useKnowledgeStore((s) => s.batchOperation);
  const entityFeedback = useKnowledgeStore((s) => s.entityFeedback);
  const loadEntityFeedback = useKnowledgeStore((s) => s.loadEntityFeedback);
  const runSynthesis = useKnowledgeStore((s) => s.runSynthesis);
  const analyzeTypes = useKnowledgeStore((s) => s.analyzeTypes);
  const fetchGraphData = useKnowledgeStore((s) => s.fetchGraphData);
  const setActivePage = useUIStore((s) => s.setActivePage);

  const [search, setSearch] = useState("");
  const [detailMode, setDetailMode] = useState<DetailMode>("view");
  const [showMerges, setShowMerges] = useState(false);
  const [showBatch, setShowBatch] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [newRelation, setNewRelation] = useState({ toId: "", type: "related_to", label: "" });
  const [synthesizing, setSynthesizing] = useState(false);

  // Load merge suggestions on mount
  useEffect(() => {
    loadPendingMerges();
  }, [loadPendingMerges]);

  // Load feedback when entity selected
  useEffect(() => {
    if (selectedEntityId) {
      loadEntityFeedback(selectedEntityId);
    }
  }, [selectedEntityId, loadEntityFeedback]);

  const filtered = useMemo(() => {
    if (!search) return entities;
    const q = search.toLowerCase();
    return entities.filter(
      (e) =>
        e.name.toLowerCase().includes(q) ||
        e.entity_type.toLowerCase().includes(q),
    );
  }, [entities, search]);

  // Group by namespace
  const grouped = useMemo(() => {
    const map = new Map<string, typeof filtered>();
    for (const e of filtered) {
      const ns = e.namespace || "default";
      if (!map.has(ns)) map.set(ns, []);
      map.get(ns)!.push(e);
    }
    return map;
  }, [filtered]);

  const handleEditSubmit = useCallback(async () => {
    if (!selectedEntityId) return;
    const updates: Record<string, unknown> = {};
    if (editing.name !== undefined) updates.name = editing.name;
    if (editing.entity_type !== undefined) updates.entity_type = editing.entity_type;
    if (editing.namespace !== undefined) updates.namespace = editing.namespace;
    if (editing.description !== undefined) updates.description = editing.description;
    if (editing.confidence !== undefined) updates.confidence = parseFloat(editing.confidence);
    await updateEntity(selectedEntityId, updates);
    setDetailMode("view");
  }, [selectedEntityId, editing, updateEntity]);

  const startEditing = useCallback(() => {
    if (!selectedEntityDetail) return;
    const e = selectedEntityDetail.entity;
    setEditing({
      name: e.name,
      entity_type: e.entity_type,
      namespace: e.namespace || "default",
      description: e.description || "",
      confidence: String(e.confidence),
    });
    setDetailMode("edit");
  }, [selectedEntityDetail]);

  const handleAddRelation = useCallback(async () => {
    if (!selectedEntityId || !newRelation.toId) return;
    const ns = selectedEntityDetail?.entity?.namespace || undefined;
    await addRelation(selectedEntityId, newRelation.toId, newRelation.type, newRelation.label || undefined, 1.0, ns);
    setNewRelation({ toId: "", type: "related_to", label: "" });
  }, [selectedEntityId, newRelation, selectedEntityDetail, addRelation]);

  const handleBatchAction = useCallback(async (action: string) => {
    if (selectedIds.size === 0) return;
    const ids = Array.from(selectedIds);
    await batchOperation(action, undefined, undefined, undefined, ids);
    setSelectedIds(new Set());
    setShowBatch(false);
  }, [selectedIds, batchOperation]);

  const toggleSelect = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const handleSynthesis = useCallback(async () => {
    setSynthesizing(true);
    try {
      await runSynthesis();
      await analyzeTypes();
      await loadPendingMerges();
      await fetchGraphData();
    } finally {
      setSynthesizing(false);
    }
  }, [runSynthesis, analyzeTypes, loadPendingMerges, fetchGraphData]);

  const entity = selectedEntityDetail?.entity;
  const inbound = selectedEntityDetail?.inbound_relations ?? [];
  const outbound = selectedEntityDetail?.outbound_relations ?? [];

  return (
    <div className={styles.browser}>
      {/* Sidebar */}
      <div className={styles.sidebar}>
        <div className={styles.sidebarHeader}>
          <input
            className={styles.search}
            placeholder="搜索实体..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <div className={styles.sidebarActions}>
            <button
              className={`${styles.toolBtn} ${showBatch ? styles.active : ""}`}
              onClick={() => setShowBatch(!showBatch)}
              title="批量操作"
            >
              批
            </button>
            <button
              className={`${styles.toolBtn} ${showMerges ? styles.active : ""}`}
              onClick={() => { setShowMerges(!showMerges); if (!showMerges) loadPendingMerges(); }}
              title="合并建议"
            >
              合
              {pendingMerges.length > 0 && <span className={styles.badgeCount}>{pendingMerges.length}</span>}
            </button>
          </div>
        </div>

        {showBatch && (
          <div className={styles.batchBar}>
            <div className={styles.batchHint}>{selectedIds.size} 个选中</div>
            <div className={styles.batchBtns}>
              <button onClick={() => handleBatchAction("hide")} title="隐藏">隐藏</button>
              <button onClick={() => handleBatchAction("show")} title="显示">显示</button>
              <button onClick={() => handleBatchAction("delete")} title="删除" className={styles.dangerBtn}>删除</button>
            </div>
          </div>
        )}

        <div className={styles.entityList}>
          {Array.from(grouped.entries()).map(([ns, ents]) => (
            <div key={ns}>
              <div className={styles.nsHeader}>{ns}</div>
              {ents.map((e) => (
                <div
                  key={e.id}
                  className={`${styles.entity} ${selectedEntityId === e.id ? styles.selected : ""} ${e.hidden ? styles.hiddenEntity : ""}`}
                  onClick={() => { selectEntity(e.id); setDetailMode("view"); }}
                >
                  {showBatch && (
                    <input
                      type="checkbox"
                      checked={selectedIds.has(e.id)}
                      onChange={() => toggleSelect(e.id)}
                      className={styles.checkbox}
                      onClick={(ev) => ev.stopPropagation()}
                      title={`选择 ${e.name}`}
                    />
                  )}
                  <span className={styles.entityDot} style={{ background: getTypeColor(e.entity_type) }} />
                  <span className={styles.entityName}>{e.name}</span>
                </div>
              ))}
            </div>
          ))}
          {filtered.length === 0 && <div className={styles.emptyHint}>暂无实体数据</div>}
        </div>

        {/* Merge suggestions panel at bottom of sidebar */}
        {showMerges && (
          <div className={styles.mergePanel}>
            <div className={styles.mergeTitle}>
              合并建议
              <button className={styles.mergeRefresh} onClick={() => { loadPendingMerges(); }}>↻</button>
            </div>
            {pendingMerges.length === 0 ? (
              <div className={styles.mergeEmpty}>暂无待合并项</div>
            ) : (
              pendingMerges.map((m: MergeSuggestion) => (
                <div key={m.id} className={styles.mergeItem}>
                  <div className={styles.mergeType}>{m.category === "entity_type" ? "类型" : "关系"}</div>
                  <div className={styles.mergeDetail}>
                    <span className={styles.mergeCanon}>{m.canonical_suggestion}</span>
                    <span className={styles.mergeSimilar}>{JSON.parse(m.similar_types || "[]").join(", ")}</span>
                  </div>
                  <div className={styles.mergeActions}>
                    <button onClick={() => confirmMerge(m.id)} title="确认合并">✓</button>
                    <button onClick={() => ignoreMerge(m.id)} title="忽略">✕</button>
                  </div>
                </div>
              ))
            )}
            <div className={styles.mergeFooter}>
              <button onClick={handleSynthesis} disabled={synthesizing}>
                {synthesizing ? "合成中..." : "运行合成引擎"}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Detail Panel */}
      <div className={styles.detail}>
        {entity ? (
          <>
            <div className={styles.detailHeader}>
              {detailMode === "edit" ? (
                <input
                  className={styles.editName}
                  value={editing.name || ""}
                  onChange={(e) => setEditing((p) => ({ ...p, name: e.target.value }))}
                />
              ) : (
                <h3 className={styles.name}>{entity.name}</h3>
              )}
              <div className={styles.detailActions}>
                {detailMode === "view" ? (
                  <button className={styles.editBtn} onClick={startEditing}>编辑</button>
                ) : (
                  <>
                    <button className={styles.saveBtn} onClick={handleEditSubmit}>保存</button>
                    <button className={styles.cancelBtn} onClick={() => setDetailMode("view")}>取消</button>
                  </>
                )}
              </div>
            </div>

            {detailMode === "edit" ? (
              /* ── Edit Form ── */
              <div className={styles.editForm}>
                <label className={styles.field}>
                  类型
                  <select value={editing.entity_type || ""} onChange={(e) => setEditing((p) => ({ ...p, entity_type: e.target.value }))} title="实体类型">
                    {[...new Set(entities.map((e) => e.entity_type))].sort().map((t) => (
                      <option key={t} value={t}>{t}</option>
                    ))}
                  </select>
                </label>
                <label className={styles.field}>
                  命名空间
                  <input value={editing.namespace || ""} onChange={(e) => setEditing((p) => ({ ...p, namespace: e.target.value }))} />
                </label>
                <label className={styles.field}>
                  描述
                  <textarea value={editing.description || ""} onChange={(e) => setEditing((p) => ({ ...p, description: e.target.value }))} rows={3} />
                </label>
                <label className={styles.field}>
                  置信度 (0-1)
                  <input type="number" min="0" max="1" step="0.05" value={editing.confidence || "0"} onChange={(e) => setEditing((p) => ({ ...p, confidence: e.target.value }))} />
                </label>
              </div>
            ) : (
              /* ── View Mode ── */
              <>
                <div className={styles.meta}>
                  <span className={styles.badge} style={{ background: `${getTypeColor(entity.entity_type)}18`, color: getTypeColor(entity.entity_type) }}>
                    {entity.entity_type}
                  </span>
                  <span>置信度 {(entity.confidence * 100).toFixed(0)}%</span>
                  <span>{entity.namespace || "default"}</span>
                  {entity.hidden && <span className={styles.hiddenTag}>已隐藏</span>}
                </div>
                <p className={styles.desc}>{entity.description || "暂无描述"}</p>
                {entity.properties && Object.keys(entity.properties).length > 0 && (
                  <div className={styles.props}>
                    {Object.entries(entity.properties).map(([key, val]: [string, any]) => (
                      <span key={key} className={styles.propTag} title={JSON.stringify(val)}>
                        {key}: {typeof val?.value === "object" ? JSON.stringify(val.value) : String(val?.value ?? val)}
                      </span>
                    ))}
                  </div>
                )}
                {(entity.aliases?.length ?? 0) > 0 && (
                  <div className={styles.aliases}>别名: {entity.aliases.join(", ")}</div>
                )}
              </>
            )}

            {/* Feedback buttons */}
            <div className={styles.feedbackRow}>
              <button className={styles.fbBtn} onClick={() => submitFeedback(entity.id, entity.hidden ? "show" : "hide")}>
                {entity.hidden ? "显示" : "隐藏"}
              </button>
              <button className={styles.fbBtn} onClick={() => submitFeedback(entity.id, "boost")}>提升置信度</button>
              <button className={`${styles.fbBtn} ${styles.dangerBtn}`} onClick={() => { deleteEntity(entity.id); }}>删除</button>
            </div>

            {/* Relations */}
            <div className={styles.relatedTitle}>关联关系 ({inbound.length + outbound.length})</div>
            <div className={styles.relations}>
              {inbound.map((r) => (
                <div key={r.id} className={styles.relItem}>
                  <span className={styles.relArrow}>&larr;</span>
                  <span className={styles.relType}>{r.relation_type}</span>
                  <span className={styles.relEntity}>{r.label || r.from_id}</span>
                  <button className={styles.relDel} onClick={() => { deleteRelation(r.id); }} title="删除关系">×</button>
                </div>
              ))}
              {outbound.map((r) => (
                <div key={r.id} className={styles.relItem}>
                  <span className={styles.relArrow}>&rarr;</span>
                  <span className={styles.relType}>{r.relation_type}</span>
                  <span className={styles.relEntity}>{r.label || r.to_id}</span>
                  <button className={styles.relDel} onClick={() => { deleteRelation(r.id); }} title="删除关系">×</button>
                </div>
              ))}
            </div>

            {/* Add relation */}
            <div className={styles.addRelRow}>
              <input
                className={styles.addRelInput}
                placeholder="目标实体 ID"
                value={newRelation.toId}
                onChange={(e) => setNewRelation((p) => ({ ...p, toId: e.target.value }))}
              />
              <select value={newRelation.type} onChange={(e) => setNewRelation((p) => ({ ...p, type: e.target.value }))} title="关系类型">
                <option value="related_to">related_to</option>
                <option value="depends_on">depends_on</option>
                <option value="contains">contains</option>
                <option value="uses">uses</option>
                <option value="creates">creates</option>
                <option value="describes">describes</option>
              </select>
              <button onClick={handleAddRelation} disabled={!newRelation.toId}>+ 添加</button>
            </div>

            {/* Feedback history */}
            {entityFeedback.length > 0 && (
              <>
                <div className={`${styles.relatedTitle} ${styles.feedbackTitle}`}>反馈记录</div>
                <div className={styles.feedbackList}>
                  {entityFeedback.slice(0, 10).map((f) => (
                    <div key={f.id} className={styles.feedbackItem}>
                      <span className={styles.fbAction}>{f.action}</span>
                      <span className={styles.fbReason}>{f.reason || "-"}</span>
                      <span className={styles.fbTime}>{f.created_at?.slice(0, 10)}</span>
                    </div>
                  ))}
                </div>
              </>
            )}

            {detailMode === "view" && (
              <button className={styles.locateBtn} onClick={() => setActivePage("sphere")}>
                在球体中定位
              </button>
            )}
          </>
        ) : (
          <div className={styles.empty}>选择一个实体查看详情</div>
        )}
      </div>
    </div>
  );
}
