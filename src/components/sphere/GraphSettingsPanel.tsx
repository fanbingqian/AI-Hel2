import { useEffect, useRef, useCallback, useState } from "react";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import type { GraphSettings2D, GraphSettings3D } from "../../types/knowledge";
import { DEFAULT_GRAPH_SETTINGS_2D, DEFAULT_GRAPH_SETTINGS_3D } from "../../types/knowledge";
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

// ── 2D Settings Panel (hermes-desktop style) ─────────────

function Settings2D() {
  const g = useKnowledgeStore((s) => s.graphSettings2D);
  const update = useKnowledgeStore((s) => s.updateGraphSettings2D);
  const reset = useKnowledgeStore((s) => s.resetGraphSettings2D);
  const showOrphans = useKnowledgeStore((s) => s.showOrphans);
  const setShowOrphans = useKnowledgeStore((s) => s.setShowOrphans);
  const showFiles = useKnowledgeStore((s) => s.showFiles);
  const setShowFiles = useKnowledgeStore((s) => s.setShowFiles);

  const [expanded, setExpanded] = useState<Set<string>>(new Set(["filters", "appearance", "forces"]));

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
          <ConfigToggle label="标签" value={g.showTags} onChange={(v) => update({ showTags: v })} />
          <ConfigToggle label="附件" value={g.showAttachments} onChange={(v) => update({ showAttachments: v })} />
          <ConfigToggle label="孤立节点" value={showOrphans} onChange={setShowOrphans} />
          <ConfigToggle label="文件节点" value={showFiles} onChange={setShowFiles} />
          <ConfigSlider label="最低重要性" value={g.minImportance} min={0} max={1} step={0.05} onChange={(v) => update({ minImportance: v })} />
          <ConfigSlider label="探索深度" value={g.explorationDepth} min={1} max={3} step={1} onChange={(v) => update({ explorationDepth: v })} />

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

      {/* ── Appearance ── */}
      <SectionHeader name="appearance" label="外观" expanded={expanded.has("appearance")} onToggle={() => toggleSection("appearance")} />
      {expanded.has("appearance") && (
        <div className={styles.section}>
          <ConfigToggle label="箭头" value={g.showArrows} onChange={(v) => update({ showArrows: v })} />
          <ConfigToggle label="属性圆环" value={g.showTypeRing} onChange={(v) => update({ showTypeRing: v })} />
          <ConfigSlider label="文本透明度" value={g.textOpacity} min={0.1} max={1} step={0.05} onChange={(v) => update({ textOpacity: v })} />
          <ConfigSlider label="节点大小" value={g.nodeSize} min={1} max={5} step={0.1} onChange={(v) => update({ nodeSize: v })} />
          <ConfigSlider label="连线粗细" value={g.linkThickness} min={1} max={5} step={0.1} onChange={(v) => update({ linkThickness: v })} />
        </div>
      )}

      {/* ── Forces ── */}
      <SectionHeader name="forces" label="力度" expanded={expanded.has("forces")} onToggle={() => toggleSection("forces")} />
      {expanded.has("forces") && (
        <div className={styles.section}>
          <ConfigSlider label="向心力" value={g.centerForce} min={0} max={1} step={0.05} onChange={(v) => update({ centerForce: v })} />
          <ConfigSlider label="排斥力" value={g.repelForce} min={0} max={20} step={0.5} onChange={(v) => update({ repelForce: v })} />
          <ConfigSlider label="吸引力" value={g.attractForce} min={0} max={1} step={0.05} onChange={(v) => update({ attractForce: v })} />
          <ConfigSlider label="连线长度" value={g.linkLength} min={30} max={500} step={10} onChange={(v) => update({ linkLength: v })} />
          <ConfigSlider label="拖拽引力" value={g.dragForce} min={1} max={15} step={0.5} onChange={(v) => update({ dragForce: v })} />
        </div>
      )}

      <button type="button" className={styles.resetBtn} onClick={reset}>恢复默认</button>
    </>
  );
}

// ── 3D Settings Panel (keep existing layout) ────────────

function Settings3D() {
  const g = useKnowledgeStore((s) => s.graphSettings3D);
  const update = useKnowledgeStore((s) => s.updateGraphSettings3D);
  const reset = useKnowledgeStore((s) => s.resetGraphSettings3D);
  const showOrphans = useKnowledgeStore((s) => s.showOrphans);
  const setShowOrphans = useKnowledgeStore((s) => s.setShowOrphans);
  const showFiles = useKnowledgeStore((s) => s.showFiles);
  const setShowFiles = useKnowledgeStore((s) => s.setShowFiles);
  const showInferenceEdges = useKnowledgeStore((s) => s.showInferenceEdges);
  const setShowInferenceEdges = useKnowledgeStore((s) => s.setShowInferenceEdges);

  const mkSlider = (label: string, key: keyof GraphSettings3D, min: number, max: number, step: number) => (
    <div className={styles.field}>
      <label className={styles.fieldLabel}>{label}</label>
      <input
        type="range"
        className={styles.slider}
        min={min}
        max={max}
        step={step}
        value={g[key] as number}
        onChange={(e) => update({ [key]: parseFloat(e.target.value) } as Partial<GraphSettings3D>)}
        aria-label={label}
        title={label}
      />
      <span className={styles.value}>{g[key]}</span>
    </div>
  );

  const mkToggle = (label: string, value: boolean, onChange: (v: boolean) => void) => (
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

  return (
    <>
      <div className={styles.section}>
        <div className={styles.sectionTitle}>物理参数</div>
        {mkSlider("斥力强度", "repulsion", 100, 5000, 100)}
        {mkSlider("引力强度", "attraction", 0.001, 0.05, 0.001)}
        {mkSlider("连接距离", "linkDistance", 20, 200, 1)}
        {mkSlider("中心引力", "centering", 0.001, 0.03, 0.001)}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>显示设置</div>
        {mkToggle("显示标签", g.showLabels, (v) => update({ showLabels: v }))}
        {mkToggle("推理连线", showInferenceEdges, setShowInferenceEdges)}
        <div className={styles.field}>
          <label className={styles.fieldLabel}>显示孤岛实体</label>
          <button
            type="button"
            className={`${styles.toggle} ${showOrphans ? styles.toggleOn : styles.toggleOff}`}
            onClick={() => setShowOrphans(!showOrphans)}
            title="显示孤岛实体"
            aria-label="显示孤岛实体"
          >
            <span className={styles.toggleKnob} />
          </button>
        </div>
        <div className={styles.field}>
          <label className={styles.fieldLabel}>显示文件节点</label>
          <button
            type="button"
            className={`${styles.toggle} ${showFiles ? styles.toggleOn : styles.toggleOff}`}
            onClick={() => setShowFiles(!showFiles)}
            title="显示文件节点"
            aria-label="显示文件节点"
          >
            <span className={styles.toggleKnob} />
          </button>
        </div>
        {mkSlider("节点大小", "nodeScale", 0.5, 2.0, 0.1)}
        {mkSlider("连线透明度", "edgeOpacity", 0.1, 1.0, 0.05)}
      </div>

      <button type="button" className={styles.resetBtn} onClick={reset}>恢复默认</button>
    </>
  );
}

// ── Main Panel ──────────────────────────────────────────

export function GraphSettingsPanel() {
  const panelRef = useRef<HTMLDivElement>(null);
  const graphViewMode = useKnowledgeStore((s) => s.graphViewMode);
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

      {graphViewMode === "2d" ? <Settings2D /> : <Settings3D />}
    </div>
  );
}
