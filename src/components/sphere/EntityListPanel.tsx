import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import styles from "./EntityListPanel.module.css";

export function EntityListPanel() {
  const panelRef = useRef<HTMLDivElement>(null);
  const entities = useKnowledgeStore((s) => s.entities);
  const lintWarnings = useKnowledgeStore((s) => s.lintWarnings);
  const showEntityList = useKnowledgeStore((s) => s.showEntityList);
  const setShowEntityList = useKnowledgeStore((s) => s.setShowEntityList);
  const entityTypeFilter = useKnowledgeStore((s) => s.entityTypeFilter);
  const setEntityTypeFilter = useKnowledgeStore((s) => s.setEntityTypeFilter);
  const namespaceFilter = useKnowledgeStore((s) => s.namespaceFilter);
  const setNamespaceFilter = useKnowledgeStore((s) => s.setNamespaceFilter);
  const selectEntity = useKnowledgeStore((s) => s.selectEntity);
  const setFocusedNode = useKnowledgeStore((s) => s.setFocusedNode);
  const setDocumentViewMode = useKnowledgeStore((s) => s.setDocumentViewMode);

  const [search, setSearch] = useState("");

  const handleClose = useCallback(() => setShowEntityList(false), [setShowEntityList]);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        handleClose();
      }
    };
    const timer = setTimeout(() => document.addEventListener("mousedown", handler), 100);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("mousedown", handler);
    };
  }, [handleClose]);

  const types = useMemo(() => {
    const set = new Set<string>();
    entities.forEach((e) => { if (e.entity_type) set.add(e.entity_type); });
    return Array.from(set).sort();
  }, [entities]);

  const namespaces = useMemo(() => {
    const set = new Set<string>();
    entities.forEach((e) => { if (e.namespace) set.add(e.namespace); });
    return Array.from(set).sort();
  }, [entities]);

  const lintCountByEntity = useMemo(() => {
    const map: Record<string, number> = {};
    lintWarnings.forEach((w) => {
      if (w.entity_id) map[w.entity_id] = (map[w.entity_id] || 0) + 1;
    });
    return map;
  }, [lintWarnings]);

  const filtered = useMemo(() => {
    return entities.filter((e) => {
      if (e.hidden) return false;
      if (entityTypeFilter && e.entity_type !== entityTypeFilter) return false;
      if (namespaceFilter && e.namespace !== namespaceFilter) return false;
      if (search.trim()) {
        const q = search.toLowerCase();
        if (!e.name.toLowerCase().includes(q) && !e.entity_type.toLowerCase().includes(q)) return false;
      }
      return true;
    }).sort((a, b) => a.name.localeCompare(b.name));
  }, [entities, entityTypeFilter, namespaceFilter, search]);

  const handleClick = (entityId: string) => {
    selectEntity(entityId);
    setFocusedNode(entityId);
  };

  const handleDoubleClick = (entityId: string) => {
    selectEntity(entityId);
    setDocumentViewMode("document");
  };

  const entityTypeLabel = (t: string) => {
    const labels: Record<string, string> = {
      concept: "概念", person: "人物", organization: "组织",
      tool: "工具", event: "事件", location: "地点",
      document: "文档", project: "项目", topic: "主题",
    };
    return labels[t] || t;
  };

  return (
    <div ref={panelRef} className={styles.panel}>
      <div className={styles.header}>
        <span className={styles.title}>实体列表 ({filtered.length})</span>
        <button type="button" className={styles.closeBtn} onClick={handleClose} title="关闭" aria-label="关闭">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <div className={styles.searchBar}>
        <input
          type="text"
          className={styles.searchInput}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="搜索实体..."
          aria-label="搜索实体"
        />
      </div>

      <div className={styles.chips}>
        <button
          type="button"
          className={`${styles.chip} ${!entityTypeFilter ? styles.chipActive : ""}`}
          onClick={() => setEntityTypeFilter(null)}
        >
          全部
        </button>
        {types.map((t) => (
          <button
            key={t}
            type="button"
            className={`${styles.chip} ${entityTypeFilter === t ? styles.chipActive : ""}`}
            onClick={() => setEntityTypeFilter(entityTypeFilter === t ? null : t)}
          >
            {entityTypeLabel(t)}
          </button>
        ))}
      </div>

      {namespaces.length > 1 && (
        <div className={styles.chips}>
          <button
            type="button"
            className={`${styles.chip} ${!namespaceFilter ? styles.chipActive : ""}`}
            onClick={() => setNamespaceFilter(null)}
          >
            全部领域
          </button>
          {namespaces.map((ns) => (
            <button
              key={ns}
              type="button"
              className={`${styles.chip} ${namespaceFilter === ns ? styles.chipActive : ""}`}
              onClick={() => setNamespaceFilter(namespaceFilter === ns ? null : ns)}
            >
              {ns}
            </button>
          ))}
        </div>
      )}

      <div className={styles.body}>
        {filtered.length === 0 && (
          <div className={styles.empty}>暂无匹配实体</div>
        )}
        {filtered.map((e) => {
          const lintCount = lintCountByEntity[e.id] || 0;
          return (
            <div
              key={e.id}
              className={styles.row}
              onClick={() => handleClick(e.id)}
              onDoubleClick={() => handleDoubleClick(e.id)}
              title="单击聚焦 / 双击打开文档"
            >
              <span className={styles.entityName}>{e.name}</span>
              <span className={styles.entityType}>{entityTypeLabel(e.entity_type)}</span>
              {lintCount > 0 && <span className={styles.badge}>{lintCount}</span>}
            </div>
          );
        })}
      </div>
    </div>
  );
}
