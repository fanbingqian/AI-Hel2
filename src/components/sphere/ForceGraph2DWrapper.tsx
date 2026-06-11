import { useEffect, useRef, useState, useMemo, useCallback } from "react";
import * as d3 from "d3";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { buildGraphData, type FGNode, type FGLink } from "./graphAdapter";
import {
  createSimulation, tick, pinNode, movePinned, unpinNode,
  type SimState, type SimNode,
} from "./physics";
import styles from "./KnowledgeSphere.module.css";

// ── Visual constants ──
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
  const simRef = useRef<SimState | null>(null);
  const animFrameRef = useRef<number>(0);
  const dragIdRef = useRef<string | null>(null);
  const hoverIdRef = useRef<string | null>(null);
  const selectedIdRef = useRef<string | null>(null);
  const transformRef = useRef({ x: 0, y: 0, k: 1 });
  const [dims, setDims] = useState({ w: 800, h: 600 });
  const [detailNode, setDetailNode] = useState<any>(null);
  const [showLegend, setShowLegend] = useState(true);
  const simReadyRef = useRef(false);

  // Resize observer
  useEffect(() => {
    const el = containerRef.current; if (!el) return;
    const ro = new ResizeObserver((e) => {
      for (const ee of e) setDims({ w: ee.contentRect.width, h: ee.contentRect.height });
    });
    ro.observe(el);
    setDims({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  // Store slices
  const entities = useKnowledgeStore((s) => s.entities);
  const relations = useKnowledgeStore((s) => s.relations);
  const inferences = useKnowledgeStore((s) => s.inferences);
  const selectedId = useKnowledgeStore((s) => s.selectedEntityId);
  const selectEntity = useKnowledgeStore((s) => s.selectEntity);
  const graphSettings = useKnowledgeStore((s) => s.graphSettings2D);
  selectedIdRef.current = selectedId;

  // Build graph data
  const { nodes, links } = useMemo(() => {
    const gd = buildGraphData(entities, relations, inferences, {
      focusedNodeId: null, focusDepth: 2, showOrphans: true, showFiles: true,
      showInferenceEdges: true, nodeRelSize: 1, searchQuery: "", minImportance: 0, colorGroups: [],
    });
    const ns = gd.nodes.map((n) => ({
      ...n,
      _color: n.entityType && n.entityType !== "unknown" ? typeColor(n.entityType) : "#8b95a3",
    }));
    return { nodes: ns, links: [...gd.links, ...gd.infLinks] as FGLink[] };
  }, [entities, relations, inferences]);

  // Data key for re-simulation
  const dataKey = useMemo(() => `${nodes.length}-${links.length}`, [nodes.length, links.length]);

  // Legend
  const legend = useMemo(() => {
    const m = new Map<string, { c: string; n: number }>();
    for (const e of entities) {
      const c = typeColor(e.entity_type);
      const v = m.get(e.entity_type) || { c, n: 0 }; v.n++; m.set(e.entity_type, v);
    }
    return [...m].sort((a, b) => b[1].n - a[1].n).map(([t, v]) => ({ type: t, ...v }));
  }, [entities]);

  // ── Physics simulation + render loop ──
  useEffect(() => {
    const svg = d3.select(svgRef.current!);
    const W = dims.w, H = dims.h;
    if (W < 10 || H < 10 || nodes.length === 0) return;

    // Setup SVG
    svg.selectAll("*").remove();
    svg.attr("viewBox", [0, 0, W, H]);

    const gRoot = svg.append("g");

    // Zoom behavior (D3 handles SVG transform, we handle pan/scale)
    const zoomBehavior = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.2, 3])
      .on("zoom", (e) => {
        transformRef.current = { x: e.transform.x, y: e.transform.y, k: e.transform.k };
        gRoot.attr("transform", e.transform.toString());
      });
    svg.call(zoomBehavior);

    // Create simulation (hand-written physics)
    const s = graphSettings;
    const nodeIds = nodes.map((n) => n.id);
    const edgeArray: [string, string][] = links.map((l) => [l.source as string, l.target as string]);
    const prevNodes = simReadyRef.current ? simRef.current?.nodes : undefined;

    simRef.current = createSimulation(nodeIds, edgeArray, {
      centering: (s.centerForce || 1) * 0.006,
      repulsion: 3000 * (s.repelForce || 3),
      attraction: 0.008 * (s.attractForce || 3),
      linkDistance: 80 * (s.linkLength || 0.5),
      dragForce: s.dragForce || 8,
      alphaDecay: 0.04,
    }, W, H, prevNodes);
    simReadyRef.current = true;

    // Set node radii
    const sim = simRef.current!;
    for (const n of nodes) {
      const sn = sim.nodes.get(n.id);
      if (sn) sn.radius = 12;
    }

    // ── SVG elements ──
    const linkG = gRoot.append("g").attr("class", "links");
    const linkEl = linkG.selectAll("path").data(links).join("path")
      .attr("stroke", EDGE_COLOR)
      .attr("stroke-width", (s.linkThickness ?? 1.25) * 0.4)
      .attr("fill", "none");

    const nodeG = gRoot.append("g").attr("class", "nodes");
    const nodeEl = nodeG.selectAll("circle").data(nodes).join("circle")
      .attr("r", (d: any) => Math.max(5, (d._sphereRadius || 5) * 1.4) * (s.nodeSize || 0.5))
      .attr("fill", (d: any) => d._color || "#8b95a3")
      .attr("stroke", "rgba(0,0,0,0.3)")
      .attr("stroke-width", 1)
      .attr("cursor", "pointer");

    const lblG = gRoot.append("g").attr("class", "labels");
    const lblEl = lblG.selectAll("text").data(nodes).join("text")
      .attr("fill", "#aaa").attr("font-size", 5).attr("text-anchor", "middle")
      .text((d: any) => d.name.length > 12 ? d.name.slice(0, 11) + "…" : d.name);

    // ── Drag (D3 handles mouse tracking) ──
    const dragBehavior = d3.drag<any, any>()
      .on("start", (e, d) => {
        dragIdRef.current = d.id;
        const sim = simRef.current!;
        const sn = sim.nodes.get(d.id);
        if (sn) pinNode(sim, d.id, sn.x, sn.y);
      })
      .on("drag", (e, d) => {
        const world = screenToWorld(e.sourceEvent, svgRef.current!, transformRef.current);
        if (world) movePinned(simRef.current!, d.id, world.x, world.y);
      })
      .on("end", (e, d) => {
        dragIdRef.current = null;
        unpinNode(simRef.current!, d.id);
      });
    nodeEl.call(dragBehavior);

    // Hover
    nodeEl.on("mouseenter", function (e, d) {
      hoverIdRef.current = d.id;
      d3.select(this).attr("stroke", HOVER_RING).attr("stroke-width", 3);
      const related = new Set<string>();
      links.forEach((l) => {
        if (l.source === d.id) related.add(l.target as string);
        if (l.target === d.id) related.add(l.source as string);
      });
      linkEl.attr("stroke", (l: any) => l.source === d.id || l.target === d.id ? EDGE_HOVER : EDGE_DIM);
      nodeEl.attr("opacity", (n: any) => n.id === d.id || related.has(n.id) ? 1 : 0.15);
      lblEl.attr("opacity", (n: any) => n.id === d.id || related.has(n.id) ? 1 : 0.3);
    });
    nodeEl.on("mouseleave", function () {
      hoverIdRef.current = null;
      nodeEl.attr("stroke", "rgba(0,0,0,0.3)").attr("stroke-width", 1).attr("opacity", 1);
      linkEl.attr("stroke", EDGE_COLOR);
      lblEl.attr("opacity", 1);
    });
    nodeEl.on("click", (e, d) => { selectEntity(d.id); setDetailNode(d); });
    svg.on("click", (e) => {
      if (e.target === svgRef.current) { selectEntity(""); setDetailNode(null); }
    });

    // ── Render loop ──
    const draw = () => {
      const sim = simRef.current;
      if (!sim) return;

      // Physics tick
      if (!(sim.frozen && !dragIdRef.current)) {
        tick(sim, W, H);
      }

      // Update positions
      nodeEl.attr("cx", (d: any) => sim.nodes.get(d.id)?.x ?? 0)
        .attr("cy", (d: any) => sim.nodes.get(d.id)?.y ?? 0);
      lblEl.attr("x", (d: any) => sim.nodes.get(d.id)?.x ?? 0)
        .attr("y", (d: any) => (sim.nodes.get(d.id)?.y ?? 0) + 10);
      linkEl.attr("d", (d: any) => {
        const s = sim.nodes.get(d.source as string);
        const t = sim.nodes.get(d.target as string);
        return s && t ? `M${s.x},${s.y}L${t.x},${t.y}` : "";
      });

      animFrameRef.current = requestAnimationFrame(draw);
    };
    animFrameRef.current = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(animFrameRef.current);
    };
  }, [dataKey, dims]);

  // ── Update physics config live (no re-simulation) ──
  useEffect(() => {
    const sim = simRef.current;
    if (!sim) return;
    const s = graphSettings;
    sim.config.centering = (s.centerForce || 1) * 0.006;
    sim.config.repulsion = 3000 * (s.repelForce || 3);
    sim.config.attraction = 0.008 * (s.attractForce || 3);
    sim.config.linkDistance = 80 * (s.linkLength || 0.5);
    sim.config.dragForce = s.dragForce || 8;
    sim.alpha = 1.0;
    sim.frozen = false;
    sim.convergenceFrames = 0;
  }, [graphSettings]);

  // ── Visual settings live update ──
  const visRef = useRef(graphSettings);
  useEffect(() => {
    const prev = visRef.current;
    visRef.current = graphSettings;
    const svg = d3.select(svgRef.current!);
    const s = graphSettings;
    if (prev.nodeSize !== s.nodeSize) {
      svg.selectAll("circle").attr("r", (d: any) => Math.max(5, (d._sphereRadius || 5) * 1.4) * (s.nodeSize || 0.5));
    }
    if (prev.linkThickness !== s.linkThickness) {
      svg.selectAll(".links path").attr("stroke-width", (s.linkThickness ?? 1.25) * 0.4);
    }
    if (prev.textOpacity !== s.textOpacity) {
      svg.selectAll(".labels text").attr("opacity", s.textOpacity ?? 0.85);
    }
  }, [graphSettings]);

  // Selected ring update + zoom to node
  useEffect(() => {
    const svg = d3.select(svgRef.current!);
    svg.selectAll("circle")
      .attr("stroke", (d: any) => d.id === selectedId ? SEL_RING : "rgba(0,0,0,0.3)")
      .attr("stroke-width", (d: any) => d.id === selectedId ? 4 : 1);

    // Zoom to selected node
    if (selectedId && simRef.current) {
      const sim = simRef.current;
      const sn = sim.nodes.get(selectedId);
      if (sn) {
        const W = dims.w, H = dims.h;
        const tx = W / 2 - sn.x! * 2.5;
        const ty = H / 2 - sn.y! * 2.5;
        const g = svg.select("g");
        g.transition().duration(500)
          .attr("transform", `translate(${tx},${ty}) scale(2.5)`);
        transformRef.current = { x: tx, y: ty, k: 2.5 };
      }
    }
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

function screenToWorld(e: MouseEvent, svg: SVGSVGElement, t: { x: number; y: number; k: number }): { x: number; y: number } | null {
  const pt = svg.createSVGPoint();
  pt.x = e.clientX;
  pt.y = e.clientY;
  const ctm = svg.getScreenCTM();
  if (!ctm) return null;
  const p = pt.matrixTransform(ctm.inverse());
  return { x: (p.x - t.x) / t.k, y: (p.y - t.y) / t.k };
}
