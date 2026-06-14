import { useEffect, useRef, useCallback, useState } from "react";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import type { GraphSettings2D } from "../../types/knowledge";
import { DEFAULT_GRAPH_SETTINGS_2D } from "../../types/knowledge";
import styles from "./GraphSettingsPanel.module.css";

// ── Helpers ──────────────────────────────────────────────

function ConfigSlider({
  label, value, min, max, step, onChange,
}: {
  label: string; value: number; min: number; max: number; step: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className={styles.field}>
      <label className={styles.fieldLabel}>{label}</label>
      <input
        type="range"
        className={styles.slider}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        aria-label={label}
        title={label}
      />
      <span className={styles.value}>{typeof value === "number" ? value.toFixed(step < 1 ? 3 : 1) : value}</span>
    </div>
  );
}

function ConfigToggle({
  label, value, onChange,
}: {
  label: string; value: boolean; onChange: (v: boolean) => void;
}) {
  return (
    <div className={styles.field}>
      <label className={styles.fieldLabel}>{label}</label>
      <button
        type="button"
        className={`${styles.toggle} ${value ? styles.toggleOn : styles.toggleOff}`}
        onClick={() => onChange(!value)}
        title={label}
        aria-label={label}
      >
        <span className={styles.toggleKnob} />
      </button>
    </div>
  );
}

function SectionHeader({
  name, label, expanded, onToggle,
}: {
  name: string; label: string; expanded: boolean; onToggle: () => void;
}) {
  return (
    <button type="button" className={styles.sectionHeader} onClick={onToggle}>
      <span className={`${styles.chevron} ${expanded ? styles.chevronDown : ""}`}>▸</span>
      <span>{label}</span>
    </button>
  );
}

const TYPE_COLOR_DEFAULTS = [
  { type: "__file__",    label: "文档",    defaultColor: "#e8e8e8" },
  { type: "location",   label: "地名",    defaultColor: "#4CAF50" },
  { type: "organization", label: "组织",  defaultColor: "#FF9800" },
  { type: "person",     label: "人物",    defaultColor: "#E91E63" },
  { type: "natural_feature", label: "自然景观", defaultColor: "#8BC34A" },
  { type: "time",       label: "时间",    defaultColor: "#00BCD4" },
  { type: "concept",    label: "概念",    defaultColor: "#7C4DFF" },
  { type: "project",    label: "项目",    defaultColor: "#FF5722" },
  { type: "tool",       label: "工具",    defaultColor: "#2196F3" },
  { type: "inferred",   label: "推断(锁)", defaultColor: "#888888" },
];

// ── 2D Settings Panel (hermes-desktop style) ─────────────

function Settings2D() {
  const g = useKnowledgeStore((s) => s.graphSettings2D);
  const update = useKnowledgeStore((s) => s.updateGraphSettings2D);
  const reset = useKnowledgeStore((s) => s.resetGraphSettings2D);
  const showOrphans = useKnowledgeStore((s) => s.showOrphans);
  const setShowOrphans = useKnowledgeStore((s) => s.setShowOrphans);
  const showFiles = useKnowledgeStore((s) => s.showFiles);
  const setShowFiles = useKnowledgeStore((s) => s.setShowFiles);

  const saveGraphSettings = useKnowledgeStore((s) => s.saveGraphSettings);

  // Auto-save settings on every change (debounced)
  const saveTimerRef = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => {
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => saveGraphSettings(), 500);
    return () => { if (saveTimerRef.current) clearTimeout(saveTimerRef.current); };
  }, [g]);

  const [expanded, setExpanded] = useState<Set<string>>(new Set(["filters", "appearance", "forces"]));
  const [typeColorsOpen, setTypeColorsOpen] = useState(false);

  const toggleSection = (name: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const addColorGroup = () => {
    const colors = ["#8b5cf6", "#e06c75", "#61afef", "#98c379", "#e5c07b", "#56b6c2"];
    const used = new Set(g.colorGroups.map((c) => c.color));
    const color = colors.find((c) => !used.has(c)) || colors[0];
    update({
      colorGroups: [...g.colorGroups, { id: crypto.randomUUID(), name: "新分组", color, pattern: "" }],
    });
  };

  const updateColorGroup = (id: string, field: string, value: string) => {
    update({
      colorGroups: g.colorGroups.map((c) => (c.id === id ? { ...c, [field]: value } : c)),
    });
  };

  const removeColorGroup = (id: string) => {
    update({ colorGroups: g.colorGroups.filter((c) => c.id !== id) });
  };

  return (
    <>
      {/* ── Filters ── */}
      <SectionHeader name="filters" label="筛选" expanded={expanded.has("filters")} onToggle={() => toggleSection("filters")} />
      {expanded.has("filters") && (
        <div className={styles.section}>
          <div className={styles.field}>
            <label className={styles.fieldLabel}>搜索</label>
            <input
              type="text"
              className={styles.textInput}
              value={g.searchQuery}
              onChange={(e) => update({ searchQuery: e.target.value })}
              placeholder="搜索实体..."
              aria-label="搜索"
            />
          </div>
          <ConfigToggle label="孤立节点" value={showOrphans} onChange={(v) => { setShowOrphans(v); update({ showOrphans: v }); }} />
          <ConfigToggle label="文档节点" value={showFiles} onChange={(v) => { setShowFiles(v); update({ showFiles: v }); }} />
          <ConfigToggle label="社区折叠" value={g.communityMode} onChange={(v) => update({ communityMode: v })} />
          <ConfigToggle label="推断实体可新建文档" value={g.inferredCreatable ?? false} onChange={(v) => update({ inferredCreatable: v })} />

          <div className={styles.colorGroupSection}>
            <div className={styles.colorGroupHeader}>
              <span className={styles.fieldLabel}>颜色分组</span>
              <button type="button" className={styles.addBtn} onClick={addColorGroup}>+ 新建</button>
            </div>
            {g.colorGroups.map((cg) => (
              <div key={cg.id} className={styles.colorGroupRow}>
                <input
                  type="color"
                  value={cg.color}
                  className={styles.colorPicker}
                  onChange={(e) => updateColorGroup(cg.id, "color", e.target.value)}
                  title="颜色"
                  aria-label="颜色"
                />
                <input
                  type="text"
                  value={cg.name}
                  className={styles.colorGroupInput}
                  onChange={(e) => updateColorGroup(cg.id, "name", e.target.value)}
                  placeholder="名称"
                  aria-label="分组名称"
                />
                <input
                  type="text"
                  value={cg.pattern}
                  className={styles.colorGroupInput}
                  onChange={(e) => updateColorGroup(cg.id, "pattern", e.target.value)}
                  placeholder="正则表达式"
                  aria-label="正则表达式"
                />
                <button type="button" className={styles.removeBtn} onClick={() => removeColorGroup(cg.id)}>✕</button>
              </div>
            ))}
          </div>

        </div>
      )}

      {/* ── Type Colors ── */}
      <SectionHeader name="typeColors" label="类型颜色" expanded={typeColorsOpen} onToggle={() => setTypeColorsOpen(!typeColorsOpen)} />
      {typeColorsOpen && (
        <div className={styles.section}>
          <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 6 }}>
            <button type="button" className={styles.resetBtn} style={{ fontSize: 10, padding: "2px 8px" }} onClick={() => update({ typeColors: {} })}>恢复默认</button>
          </div>
          {TYPE_COLOR_DEFAULTS.map(({ type, label, defaultColor }) => {
            const current = g.typeColors?.[type] ?? defaultColor;
            return (
              <div key={type} className={styles.colorGroupRow}>
                <input
                  type="color"
                  value={current}
                  className={styles.colorPicker}
                  onChange={(e) => update({ typeColors: { ...g.typeColors, [type]: e.target.value } })}
                  title={`${label} (${type})`}
                  aria-label={label}
                />
                <span style={{ flex: 1, fontSize: 11, color: "#ccc" }}>{label} <span style={{ color: "#555", fontSize: 10 }}>{type}</span></span>
                {g.typeColors?.[type] && (
                  <button type="button" className={styles.removeBtn} onClick={() => {
                    const next = { ...g.typeColors };
                    delete next[type];
                    update({ typeColors: next });
                  }}>↩</button>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* ── Appearance ── */}
      <SectionHeader name="appearance" label="外观" expanded={expanded.has("appearance")} onToggle={() => toggleSection("appearance")} />
      {expanded.has("appearance") && (
        <div className={styles.section}>
          <ConfigToggle label="箭头" value={g.showArrows} onChange={(v) => update({ showArrows: v })} />
          <ConfigToggle label="属性圆环" value={g.showTypeRing} onChange={(v) => update({ showTypeRing: v })} />
          <ConfigSlider label="文本透明度" value={g.textOpacity} min={0.1} max={1} step={0.05} onChange={(v) => update({ textOpacity: v })} />
          <ConfigSlider label="节点大小" value={g.nodeSize} min={0.3} max={3} step={0.05} onChange={(v) => update({ nodeSize: v })} />
          <ConfigSlider label="连线粗细" value={g.linkThickness} min={0.1} max={2.5} step={0.05} onChange={(v) => update({ linkThickness: v })} />
          <ConfigSlider label="边线透明度" value={g.edgeOpacity} min={0.05} max={1} step={0.05} onChange={(v) => update({ edgeOpacity: v })} />
        </div>
      )}

      {/* ── Forces ── */}
      <SectionHeader name="forces" label="力度" expanded={expanded.has("forces")} onToggle={() => toggleSection("forces")} />
      {expanded.has("forces") && (
        <div className={styles.section}>
          <ConfigSlider label="向心力" value={g.centerForce} min={0} max={1} step={0.01} onChange={(v) => update({ centerForce: v })} />
          <ConfigSlider label="节点排斥力" value={g.repelForce} min={0} max={20} step={0.5} onChange={(v) => update({ repelForce: v })} />
          <ConfigSlider label="相连节点吸引力" value={g.attractForce} min={0} max={1} step={0.01} onChange={(v) => update({ attractForce: v })} />
          <ConfigSlider label="连线长度" value={g.linkLength} min={30} max={500} step={5} onChange={(v) => update({ linkLength: v })} />
          <ConfigSlider label="拖拽引力" value={g.dragForce} min={1} max={15} step={0.5} onChange={(v) => update({ dragForce: v })} />
        </div>
      )}

      <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
        <button type="button" className={styles.resetBtn} onClick={reset}>恢复默认</button>
      </div>
    </>
  );
}

// ── Main Panel ──────────────────────────────────────────

export function GraphSettingsPanel() {
  const panelRef = useRef<HTMLDivElement>(null);
  const saveGraphSettings = useKnowledgeStore((s) => s.saveGraphSettings);
  const setSettingsOpen = useKnowledgeStore((s) => s.setSettingsOpen);

  const close = useCallback(() => {
    saveGraphSettings();
    setSettingsOpen(false);
  }, [saveGraphSettings, setSettingsOpen]);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        close();
      }
    };
    const timer = setTimeout(() => document.addEventListener("mousedown", handler), 100);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("mousedown", handler);
    };
  }, [close]);

  return (
    <div ref={panelRef} className={styles.panel}>
      <div className={styles.header}>
        <span className={styles.title}>图谱配置</span>
        <button type="button" className={styles.closeBtn} onClick={close} title="关闭" aria-label="关闭">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <Settings2D />
    </div>
  );
}
