import { useEffect, useRef, useCallback, useState, useMemo } from "react";
import * as d3 from "d3";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { useChatStore } from "../../stores/chatStore";
import { buildGraphData, type FGNode, type FGLink } from "./graphAdapter";
import { NodeContextMenu } from "./NodeContextMenu";
import styles from "./KnowledgeSphere.module.css";

// ── MiroFish palette ──
const MIROFISH_PALETTE = [
  "#FF6B35", "#004E89", "#7B2D8E", "#1A936F", "#C5283D",
  "#E9724C", "#3498db", "#9b59b6", "#27ae60", "#f39c12",
];
const typeColorCache = new Map<string, string>();
let _pc = 0;
function typeColor(t: string) {
  const k = t.toLowerCase();
  if (!typeColorCache.has(k)) { typeColorCache.set(k, MIROFISH_PALETTE[_pc++ % MIROFISH_PALETTE.length]); }
  return typeColorCache.get(k)!;
}

const EDGE_COLOR = "#9999aa";
const EDGE_HOVER = "#b0a0d0";
const EDGE_DIM = "rgba(153,153,170,0.06)";
const SEL_RING = "#E91E63";
const HOVER_RING = "#3498db";

export function ForceGraph2DWrapper() {
  const svgRef = useRef<SVGSVGElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const simRef = useRef<d3.Simulation<any, any> | null>(null);
  const [dims, setDims] = useState({ w: 800, h: 600 });
  const [detailNode, setDetailNode] = useState<any>(null);
  const [showLegend, setShowLegend] = useState(true);
  const hoveredIdRef = useRef<string | null>(null);

  useEffect(() => {
    const el = containerRef.current; if (!el) return;
    const ro = new ResizeObserver((e) => {
      for (const ee of e) setDims({ w: ee.contentRect.width, h: ee.contentRect.height });
    });
    ro.observe(el);
    setDims({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  const entities = useKnowledgeStore((s) => s.entities);
  const relations = useKnowledgeStore((s) => s.relations);
  const inferences = useKnowledgeStore((s) => s.inferences);
  const selectedId = useKnowledgeStore((s) => s.selectedEntityId);
  const selectEntity = useKnowledgeStore((s) => s.selectEntity);
  const graphSettings = useKnowledgeStore((s) => s.graphSettings2D);

  // Build graph data
  const { nodes, links } = useMemo(() => {
    const gd = buildGraphData(entities, relations, inferences, {
      focusedNodeId: null, focusDepth: 2,
      showOrphans: true, showFiles: true, showInferenceEdges: true,
      nodeRelSize: 1, searchQuery: "", minImportance: 0, colorGroups: [],
    });
    // Assign colors
    const ns = gd.nodes.map((n) => ({
      ...n,
      _color: n.entityType && n.entityType !== "unknown" ? typeColor(n.entityType) : "#8b95a3",
    }));
    const ls = [...gd.links, ...gd.infLinks].map((l) => ({
      ...l,
      _isInf: (l as any).relationTypes?.includes?.("__inference__"),
    }));
    return { nodes: ns, links: ls };
  }, [entities, relations, inferences]);

  // Type legend
  const legend = useMemo(() => {
    const m = new Map<string, { c: string; n: number }>();
    for (const e of entities) {
      const c = typeColor(e.entity_type);
      const v = m.get(e.entity_type) || { c, n: 0 }; v.n++; m.set(e.entity_type, v);
    }
    return [...m].sort((a, b) => b[1].n - a[1].n).map(([t, v]) => ({ type: t, ...v }));
  }, [entities]);

  // Only rebuild simulation when data changes (not on resize)
  const dataKey = useMemo(() => `${nodes.length}-${links.length}`, [nodes.length, links.length]);

  useEffect(() => {
    const svg = d3.select(svgRef.current!);
    const W = dims.w, H = dims.h;
    if (W < 10 || H < 10) return;

    // Setup SVG once — don't clear on resize if sim already exists
    const hasExistingSim = simRef.current !== null;
    if (!hasExistingSim) {
      svg.selectAll("*").remove();
    }
    svg.attr("viewBox", [0, 0, W, H]);

    if (hasExistingSim) {
      // Just update center and viewBox — no rebuild
      simRef.current!.force("center", d3.forceCenter(W / 2, H / 2).strength(0.15));
      return;
    }

    const gRoot = svg.append("g");

    // Zoom
    svg.call(
      d3.zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.1, 5])
        .on("zoom", (e) => gRoot.attr("transform", e.transform.toString()))
    );

    // Clone data for simulation
    const simNodes: any[] = nodes.map((n) => ({ ...n, x: W / 2 + (Math.random() - 0.5) * 100, y: H / 2 + (Math.random() - 0.5) * 100 }));
    const simLinks = links.map((l) => {
      const s = typeof l.source === "object" ? (l.source as any).id : l.source;
      const t = typeof l.target === "object" ? (l.target as any).id : l.target;
      const sn = simNodes.find((n) => n.id === s);
      const tn = simNodes.find((n) => n.id === t);
      return { ...l, source: sn || s, target: tn || t };
    });

    // Force simulation
    const sim = d3.forceSimulation(simNodes)
      .force("link", d3.forceLink(simLinks).id((d: any) => d.id).distance(40))
      .force("charge", d3.forceManyBody().strength(-180))
      .force("center", d3.forceCenter(W / 2, H / 2).strength(0.15))
      .force("collide", d3.forceCollide(15))
      .alphaDecay(0.03);
    simRef.current = sim;

    // ── Links ──
    const linkG = gRoot.append("g").attr("class", "links");
    const linkEl = linkG.selectAll("path").data(simLinks).join("path")
      .attr("stroke", EDGE_COLOR)
      .attr("stroke-width", 0.6)
      .attr("fill", "none");

    // Link labels
    const lblG = gRoot.append("g").attr("class", "link-labels");
    const lblEl = lblG.selectAll("g").data(simLinks).join("g");
    lblEl.append("rect").attr("fill", "rgba(26,26,26,0.85)").attr("rx", 3);
    lblEl.append("text").attr("fill", "#888").attr("font-size", 8).attr("text-anchor", "middle").attr("dy", 3)
      .text((d: any) => {
        if (d._isInf || d.isFileEdge) return "";
        const ts = Array.isArray(d.relationTypes) ? d.relationTypes : [d.relationTypes];
        return ts[0] || "";
      });

    // ── Nodes ──
    const nodeG = gRoot.append("g").attr("class", "nodes");
    const nodeEl = nodeG.selectAll("circle").data(simNodes).join("circle")
      .attr("r", (d: any) => Math.max(5, (d._sphereRadius || 5) * 1.4))
      .attr("fill", (d: any) => d._color || "#8b95a3")
      .attr("stroke", "rgba(0,0,0,0.3)")
      .attr("stroke-width", 1)
      .attr("cursor", "pointer")
      .call(
        d3.drag<any, any>()
          .on("start", (e, d) => { if (!e.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; })
          .on("drag", (e, d) => { d.fx = e.x; d.fy = e.y; })
          .on("end", (e, d) => { if (!e.active) sim.alphaTarget(0); d.fx = null; d.fy = null; })
      );

    // Node labels
    const nlG = gRoot.append("g").attr("class", "node-labels");
    const nlEl = nlG.selectAll("text").data(simNodes).join("text")
      .attr("fill", "#aaa").attr("font-size", 10).attr("font-weight", 600)
      .attr("text-anchor", "middle")
      .attr("dy", (d: any) => Math.max(5, (d._sphereRadius || 5) * 1.4) + 12)
      .text((d: any) => d.name.length > 12 ? d.name.slice(0, 11) + "…" : d.name);

    // Hover / click events
    nodeEl
      .on("mouseenter", function (e, d) {
        hoveredIdRef.current = d.id;
        d3.select(this).attr("stroke", HOVER_RING).attr("stroke-width", 3);
        const related = new Set<string>();
        simLinks.forEach((l: any) => {
          if (l.source.id === d.id) related.add(l.target.id);
          if (l.target.id === d.id) related.add(l.source.id);
        });
        linkEl.attr("stroke", (l: any) => l.source.id === d.id || l.target.id === d.id ? EDGE_HOVER : EDGE_DIM);
        nodeEl.attr("opacity", (n: any) => n.id === d.id || related.has(n.id) ? 1 : 0.15);
        nlEl.attr("opacity", (n: any) => n.id === d.id || related.has(n.id) ? 1 : 0.2);
      })
      .on("mouseleave", function () {
        hoveredIdRef.current = null;
        nodeEl.attr("stroke", "rgba(0,0,0,0.3)").attr("stroke-width", 1).attr("opacity", 1);
        linkEl.attr("stroke", EDGE_COLOR);
        nlEl.attr("opacity", 1);
      })
      .on("click", (e, d) => { selectEntity(d.id); setDetailNode(d); });

    svg.on("click", (e) => {
      if (e.target === svgRef.current) { selectEntity(""); setDetailNode(null); }
    });

    // Tick
    sim.on("tick", () => {
      linkEl.attr("d", (d: any) => `M${d.source.x},${d.source.y}L${d.target.x},${d.target.y}`);
      lblEl.each(function (d: any) {
        const mx = (d.source.x + d.target.x) / 2, my = (d.source.y + d.target.y) / 2;
        const t = d3.select(this).select("text").text();
        if (t) {
          const w = t.length * 5 + 8, h = 14;
          d3.select(this).select("rect").attr("x", mx - w / 2).attr("y", my - h / 2).attr("width", w).attr("height", h);
          d3.select(this).select("text").attr("x", mx).attr("y", my);
        }
      });
      nodeEl.attr("cx", (d: any) => d.x).attr("cy", (d: any) => d.y);
      nlEl.attr("x", (d: any) => d.x).attr("y", (d: any) => d.y);
    });

    return () => { sim.stop(); simRef.current = null; };
  }, [dataKey, dims, selectEntity]);

  // Selected node ring
  useEffect(() => {
    const s = d3.select(svgRef.current!);
    s.selectAll("circle").attr("stroke", (d: any) => {
      if (d.id === selectedId) return SEL_RING;
      return "rgba(0,0,0,0.3)";
    }).attr("stroke-width", (d: any) => d.id === selectedId ? 4 : 1);
  }, [selectedId]);

  const detailEntity = useMemo(() => {
    if (!detailNode) return null;
    const e = entities.find((en) => en.id === detailNode.id);
    if (!e) return null;
    const rels = relations.filter((r) => r.from_id === e.id || r.to_id === e.id).slice(0, 10);
    return { entity: e, relations: rels };
  }, [detailNode, entities, relations]);

  return (
    <div ref={containerRef} className={styles.container}>
      <svg ref={svgRef} style={{ width: "100%", height: "100%", display: "block" }} />

      {/* Legend */}
      {showLegend && legend.length > 0 && (
        <div className={styles.legendPanel} style={{
          position: "absolute", top: 8, left: 8, zIndex: 10,
          background: "rgba(16,20,28,0.92)", borderRadius: 8,
          border: "1px solid rgba(255,255,255,0.08)", padding: "8px 12px",
          fontSize: 11, color: "#ccc", maxHeight: "40%", overflowY: "auto",
        }}>
          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
            <span style={{ fontWeight: 600, fontSize: 12 }}>图例</span>
            <button onClick={() => setShowLegend(false)} style={{ background: "none", border: "none", color: "#666", cursor: "pointer", fontSize: 14 }}>×</button>
          </div>
          {legend.map((t) => (
            <div key={t.type} style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 3 }}>
              <span style={{ width: 10, height: 10, borderRadius: "50%", background: t.c, flexShrink: 0 }} />
              <span style={{ flex: 1 }}>{t.type}</span>
              <span style={{ color: "#666", fontSize: 10 }}>{t.n}</span>
            </div>
          ))}
        </div>
      )}
      {!showLegend && (
        <button onClick={() => setShowLegend(true)} style={{ position: "absolute", top: 8, left: 8, zIndex: 10,
          background: "rgba(16,20,28,0.8)", border: "1px solid rgba(255,255,255,0.1)",
          borderRadius: 6, color: "#888", cursor: "pointer", fontSize: 11, padding: "4px 8px" }}>图例</button>
      )}

      {/* Detail panel */}
      {detailNode && detailEntity && (
        <div className={styles.detailPanel} style={{
          position: "absolute", top: 48, right: 12, zIndex: 10,
          background: "rgba(16,20,28,0.94)", borderRadius: 10,
          border: "1px solid rgba(255,255,255,0.08)", padding: "14px 16px",
          width: 260, maxHeight: "60%", overflowY: "auto", color: "#ccc", fontSize: 12,
        }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
            <span style={{ fontWeight: 600, fontSize: 14 }}>{detailEntity.entity.name}</span>
            <span style={{ fontSize: 10, padding: "2px 8px", borderRadius: 4, background: typeColor(detailEntity.entity.entity_type), color: "#fff" }}>
              {detailEntity.entity.entity_type}
            </span>
            <button onClick={() => setDetailNode(null)} style={{ background: "none", border: "none", color: "#666", cursor: "pointer", fontSize: 16 }}>×</button>
          </div>
          {detailEntity.entity.description && (
            <div style={{ marginBottom: 8, color: "#999", lineHeight: 1.5 }}>{detailEntity.entity.description}</div>
          )}
          {detailEntity.relations.length > 0 && (
            <div>
              <div style={{ fontWeight: 600, marginBottom: 4, color: "#888", fontSize: 11 }}>关联 ({detailEntity.relations.length})</div>
              {detailEntity.relations.map((r, i) => (
                <div key={i} style={{ padding: "3px 6px", marginBottom: 2, background: "rgba(255,255,255,0.03)", borderRadius: 4 }}>
                  → {r.to_id === detailEntity.entity.id ? r.from_id : r.to_id}
                  <span style={{ color: "#888", marginLeft: 6 }}>[{r.relation_type}]</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
