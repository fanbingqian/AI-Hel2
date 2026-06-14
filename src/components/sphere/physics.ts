// ── Force-directed physics engine ──
// Based on Obsidian's worker.js (D3-force + Barnes-Hut via WASM/fallback JS).
// Extracted from Obsidian 1.9.x ASAR at D:/obsidian黑曜石/resources/obsidian.asar
//
// Obsidian's force configuration (from source):
//   forceX / forceY   — per-node pull toward center, strength = centerForce (0.1)
//   forceManyBody     — Barnes-Hut O(n log n), strength = -(repelForce³) = -1000
//                       theta = 0.9, distanceMin = 30
//   forceLink         — Hooke springs, strength = 1/min(deg), distance = 250
//   forceCollide      — circle collision, radius = 60, strength = 0.5
//
// D3 constants (from Obsidian source):
//   velocityDecay = 0.4  (vx *= 0.6)
//   alphaDecay ≈ 0.0228  (alpha *= 0.9772; we approximate as 0.98)
//   alphaMin = 0.001

export interface SimNode {
  id: string;       x: number;      y: number;
  vx: number;       vy: number;
  pinned: boolean;  radius: number;
}

export interface SimEdge {
  source: string;   target: string;  weight: number;
}

export interface SimConfig {
  centerStrength: number;   // per-node radial pull (Obsidian forceX/forceY)
  repelBase: number;         // repelForce³ (Obsidian: 1000 = 10³)
  linkStrength: number;      // spring stiffness multiplier (Obsidian: 1.0)
  linkDistance: number;      // ideal edge length px (Obsidian: 250)
  dragBoost: number;         // spring multiplier while dragging
}

export const DEFAULT_SIM_CONFIG: SimConfig = {
  centerStrength: 0.1,
  repelBase:      1000,
  linkStrength:   1.0,
  linkDistance:   250,
  dragBoost:      4,
};

// ── D3 constants (from Obsidian worker.js) ──
const VELOCITY_DECAY     = 0.4;
const ALPHA_DECAY_FACTOR = 0.9772;  // Obsidian: 1 - 0.001^(1/300) ≈ 0.0228 decay
const ALPHA_MIN          = 0.001;
const SPEED_THRESHOLD    = 0.015;
const ALPHA_THRESHOLD    = 0.03;
const FREEZE_FRAMES      = 12;

// ── forceManyBody constants (from Obsidian source) ──
const BH_THETA    = 0.9;      // Barnes-Hut opening angle
const BH_THETA_SQ = 0.81;     // theta²
const DISTANCE_MIN = 30;      // force capping at very close range (prevents explosion)

// ── forceCollide constants (from Obsidian source) ──
const COLLIDE_STRENGTH = 0.5;

export interface SimState {
  nodes: Map<string, SimNode>;
  edges: SimEdge[];
  alpha: number;
  frozen: boolean;
  convergenceFrames: number;
  config: SimConfig;
}

// ═══════════════════════════════════════════════════════════════
// Barnes-Hut Quadtree (Obsidian: D3 quadtree)
// ═══════════════════════════════════════════════════════════════

interface QTreeNode {
  x: number; y: number;        // center of mass
  mass: number;                 // total mass
  x0: number; y0: number;      // cell bounds
  x1: number; y1: number;
  children?: QTreeNode[];
}

function qtreeBuild(nodes: SimNode[]): QTreeNode | null {
  if (nodes.length === 0) return null;
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const n of nodes) {
    if (n.x < x0) x0 = n.x; if (n.y < y0) y0 = n.y;
    if (n.x > x1) x1 = n.x; if (n.y > y1) y1 = n.y;
  }
  const pad = Math.max(x1 - x0, y1 - y0, 1) * 0.01;
  x0 -= pad; y0 -= pad; x1 += pad; y1 += pad;
  return qtreeInsert(null, nodes, x0, y0, x1, y1);
}

function qtreeInsert(
  _root: QTreeNode | null,
  nodes: SimNode[],
  x0: number, y0: number, x1: number, y1: number,
): QTreeNode {
  if (nodes.length === 1) {
    const n = nodes[0];
    return { x: n.x, y: n.y, mass: 1, x0, y0, x1, y1 };
  }
  let cx = 0, cy = 0;
  for (const n of nodes) { cx += n.x; cy += n.y; }
  cx /= nodes.length; cy /= nodes.length;

  const xm = (x0 + x1) / 2, ym = (y0 + y1) / 2;
  const quads: SimNode[][] = [[], [], [], []];
  for (const n of nodes) {
    const i = (n.x >= xm ? 1 : 0) | (n.y >= ym ? 2 : 0);
    quads[i].push(n);
  }

  // Index: (x>=xm?1:0)|(y>=ym?2:0) → 0=NW,1=NE,2=SW,3=SE
  const children: QTreeNode[] = [];
  const bounds: [number, number, number, number][] = [
    [x0, y0, xm, ym], [xm, y0, x1, ym], [x0, ym, xm, y1], [xm, ym, x1, y1],
  ];
  for (let i = 0; i < 4; i++) {
    if (quads[i].length > 0) {
      const [cx0, cy0, cx1, cy1] = bounds[i];
      children.push(qtreeInsert(null, quads[i], cx0, cy0, cx1, cy1));
    }
  }
  return { x: cx, y: cy, mass: nodes.length, x0, y0, x1, y1, children };
}

function qtreeApply(node: QTreeNode, body: SimNode, repelK: number): void {
  if (node.mass === 0) return;
  if (node.children === undefined && node.x === body.x && node.y === body.y) return;

  let dx = node.x - body.x, dy = node.y - body.y;
  let dist = Math.sqrt(dx * dx + dy * dy) || 1;

  // Obsidian: distanceMin prevents force explosion at very close range
  if (dist < DISTANCE_MIN) dist = DISTANCE_MIN;

  const cellW = node.x1 - node.x0;
  if (node.children === undefined || (cellW * cellW) / (dist * dist) < BH_THETA_SQ) {
    const rf = repelK * node.mass / dist;
    body.vx -= (dx / dist) * rf;
    body.vy -= (dy / dist) * rf;
  } else {
    for (const child of node.children!) qtreeApply(child, body, repelK);
  }
}

// ═══════════════════════════════════════════════════════════════
// Simulation
// ═══════════════════════════════════════════════════════════════

export function createSimulation(
  nodeIds: string[],
  edges: [string, string, number?][],
  config: Partial<SimConfig> = {},
  width: number,
  height: number,
  existingNodes?: Map<string, { x: number; y: number; radius: number }>,
): SimState {
  const cfg = { ...DEFAULT_SIM_CONFIG, ...config };
  const nodes = new Map<string, SimNode>();
  const cx = width / 2, cy = height / 2;
  const spread = Math.min(width, height) * 0.25;

  for (const id of nodeIds) {
    const prev = existingNodes?.get(id);
    nodes.set(id, prev ? {
      id, x: prev.x + (Math.random() - 0.5) * 4,
      y: prev.y + (Math.random() - 0.5) * 4,
      vx: 0, vy: 0, pinned: false, radius: prev.radius,
    } : {
      id,
      x: cx + Math.sqrt(Math.random()) * spread * Math.cos(Math.random() * 2 * Math.PI),
      y: cy + Math.sqrt(Math.random()) * spread * Math.sin(Math.random() * 2 * Math.PI),
      vx: 0, vy: 0, pinned: false, radius: 8,
    });
  }

  return {
    nodes,
    edges: edges.map(([s, t, w]) => ({ source: s, target: t, weight: w ?? 1.0 })),
    alpha: 1.0, frozen: false, convergenceFrames: 0, config: cfg,
  };
}

export function tick(state: SimState, width: number, height: number): void {
  const { nodes, edges, config } = state;
  const arr = Array.from(nodes.values());
  const n = arr.length;
  if (n === 0) return;

  const cx = width / 2, cy = height / 2;
  const dragId = Array.from(nodes.entries()).find(([,v]) => v.pinned)?.[0] || null;

  // ═══════════════════════════════════════════════════════════
  // ① CENTER: centroid translation (always active, immediate feedback)
  //    Our addition — Obsidian doesn't have this.  It provides
  //    instant visual response when user adjusts centerForce slider
  //    even after the velocity simulation has frozen.
  // ═══════════════════════════════════════════════════════════
  if (config.centerStrength > 0) {
    let sx = 0, sy = 0, cnt = 0;
    for (const nd of arr) { if (!nd.pinned) { sx += nd.x; sy += nd.y; cnt++; } }
    if (cnt > 0) {
      const tx = (cx - sx / cnt) * config.centerStrength;
      const ty = (cy - sy / cnt) * config.centerStrength;
      for (const nd of arr) { if (!nd.pinned) { nd.x += tx; nd.y += ty; } }
    }
  }

  if (state.frozen) return;
  const alpha = state.alpha;

  // ═══════════════════════════════════════════════════════════
  // ② CENTER: per-node radial pull (Obsidian: forceX + forceY)
  //    Pulls every node toward viewport center.
  //    Strength = centerStrength (default 0.1).
  //    This is the FORCE part — separate from the always-active
  //    centroid translation above.
  // ═══════════════════════════════════════════════════════════
  const centerK = config.centerStrength * alpha;
  for (const nd of arr) {
    if (nd.pinned) continue;
    nd.vx += (cx - nd.x) * centerK;
    nd.vy += (cy - nd.y) * centerK;
  }

  // ═══════════════════════════════════════════════════════════
  // ③ REPULSION: Barnes-Hut (Obsidian: forceManyBody)
  //    strength = -(repelForce³), default -1000
  //    theta = 0.9, distanceMin = 30
  // ═══════════════════════════════════════════════════════════
  const repelK = config.repelBase * alpha;
  const root = qtreeBuild(arr);
  if (root) {
    for (const nd of arr) {
      if (nd.pinned) continue;
      qtreeApply(root, nd, repelK);
    }
  }

  // ═══════════════════════════════════════════════════════════
  // ④ COLLISION: circle overlap prevention (Obsidian: forceCollide)
  //    radius = per-node, strength = 0.5
  // ═══════════════════════════════════════════════════════════
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      const a = arr[i], b = arr[j];
      const dx = b.x - a.x, dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const minR = a.radius + b.radius + 4;
      if (dist < minR) {
        const overlap = (minR - dist) * COLLIDE_STRENGTH;
        if (!a.pinned) { a.x -= (dx / dist) * overlap; a.y -= (dy / dist) * overlap; }
        if (!b.pinned) { b.x += (dx / dist) * overlap; b.y += (dy / dist) * overlap; }
      }
    }
  }

  // ═══════════════════════════════════════════════════════════
  // ⑤ SPRINGS: Hooke law (Obsidian: forceLink)
  //    Obsidian strength = linkStrength / min(deg(s), deg(t))
  //    This makes hub nodes have weaker per-edge springs, letting
  //    the graph breathe naturally instead of collapsing.
  // ═══════════════════════════════════════════════════════════
  // Pre-compute node degrees
  const degree = new Map<string, number>();
  for (const e of edges) {
    degree.set(e.source, (degree.get(e.source) || 0) + 1);
    degree.set(e.target, (degree.get(e.target) || 0) + 1);
  }

  const baseLen = config.linkDistance;
  const baseK = config.linkStrength * alpha;

  for (const e of edges) {
    const sa = nodes.get(e.source), sb = nodes.get(e.target);
    if (!sa || !sb) continue;
    const degS = degree.get(e.source) || 1;
    const degT = degree.get(e.target) || 1;
    // Obsidian degree-based: nodes with many connections have weaker springs
    const springK = baseK / Math.min(degS, degT);

    let dx = sb.x - sa.x, dy = sb.y - sa.y;
    let dist = Math.sqrt(dx * dx + dy * dy) || 1;
    const wf = 0.5 + e.weight * 0.5;
    const ideal = baseLen / Math.max(wf, 0.55);
    let sf = (dist - ideal) * springK;
    if (dragId && (e.source === dragId || e.target === dragId)) sf *= config.dragBoost;
    if (!sa.pinned) { sa.vx += (dx / dist) * sf; sa.vy += (dy / dist) * sf; }
    if (!sb.pinned) { sb.vx -= (dx / dist) * sf; sb.vy -= (dy / dist) * sf; }
  }

  // ═══════════════════════════════════════════════════════════
  // ⑥ VELOCITY INTEGRATION: D3-style (Obsidian: vx *= 0.6)
  // ═══════════════════════════════════════════════════════════
  const maxV = 8 + alpha * 25;
  let peak = 0;
  for (const nd of arr) {
    if (nd.pinned) continue;
    const sp = Math.sqrt(nd.vx * nd.vx + nd.vy * nd.vy);
    if (sp > maxV) { nd.vx = (nd.vx / sp) * maxV; nd.vy = (nd.vy / sp) * maxV; }
    nd.vx *= (1 - VELOCITY_DECAY);
    nd.vy *= (1 - VELOCITY_DECAY);
    nd.x += nd.vx;
    nd.y += nd.vy;
    const sp2 = Math.sqrt(nd.vx * nd.vx + nd.vy * nd.vy);
    if (sp2 > peak) peak = sp2;
  }

  // ═══════════════════════════════════════════════════════════
  // ⑦ ALPHA DECAY + CONVERGENCE (Obsidian: alphaTarget decay)
  // ═══════════════════════════════════════════════════════════
  if (dragId) {
    if (state.alpha < 0.05) state.alpha = 0.05;
    if (state.alpha < 0.15) state.alpha += 0.01;
    state.alpha *= 0.985;
    state.frozen = false;
    state.convergenceFrames = 0;
  } else if (!state.frozen) {
    state.alpha *= ALPHA_DECAY_FACTOR;
  }

  if (!dragId) {
    if (state.alpha <= ALPHA_MIN) {
      state.frozen = true; state.alpha = 0;
    } else if (peak < SPEED_THRESHOLD && state.alpha < ALPHA_THRESHOLD) {
      state.convergenceFrames++;
      if (state.convergenceFrames >= FREEZE_FRAMES) {
        state.frozen = true; state.alpha = 0;
      }
    } else if (peak >= SPEED_THRESHOLD) {
      state.convergenceFrames = 0;
    }
  }
}

export function pinNode(s: SimState, id: string, x: number, y: number): void {
  const n = s.nodes.get(id); if (!n) return;
  n.pinned = true; n.x = x; n.y = y; n.vx = 0; n.vy = 0;
  s.frozen = false; s.convergenceFrames = 0;
}
export function movePinned(s: SimState, id: string, x: number, y: number): void {
  const n = s.nodes.get(id); if (!n?.pinned) return;
  n.x = x; n.y = y;
}
export function unpinNode(s: SimState, id: string): void {
  const n = s.nodes.get(id); if (n) n.pinned = false;
}
export function setConfig(s: SimState, p: Partial<SimConfig>): void {
  Object.assign(s.config, p);
}
