// ── Hand-written force-directed physics engine ──
// Matches hermes-desktop Knowledge.tsx physics parameters.
// Coulomb repulsion O(n²) + Hooke springs + collision detection +
// centering + convergence freeze + drag support.

export interface SimNode {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  pinned: boolean;
  radius: number;
}

export interface SimEdge {
  source: string;
  target: string;
}

export interface SimConfig {
  centering: number;
  repulsion: number;
  attraction: number;
  linkDistance: number;
  dragForce: number;
  alphaDecay: number;
}

export const DEFAULT_SIM_CONFIG: SimConfig = {
  centering: 0.006,   // baseCenter × centerForce (1.0 default)
  repulsion: 3000,    // baseRepel
  attraction: 0.008,  // baseSpring
  linkDistance: 80,   // baseEdgeLen
  dragForce: 8,       // hermes-desktop default
  alphaDecay: 0.04,   // 1 - 0.96 hermes-desktop alphaDecay
};

export interface SimState {
  nodes: Map<string, SimNode>;
  edges: SimEdge[];
  alpha: number;
  frozen: boolean;
  convergenceFrames: number;
  config: SimConfig;
}

// ── Hermes-desktop physics constants ──

const CONVERGENCE_SPEED_THRESHOLD = 0.02;
const CONVERGENCE_ALPHA_THRESHOLD = 0.05;
const CONVERGENCE_FRAMES_NEEDED = 8;
const BOUNDARY_RADIUS_RATIO = 0.48;
const ALPHA_MIN = 0.001;
const ALPHA_DECAY_FACTOR = 0.96; // per-frame multiplier (hermes-desktop)

export function createSimulation(
  nodeIds: string[],
  edges: [string, string][],
  config: Partial<SimConfig> = {},
  width: number,
  height: number,
  existingNodes?: Map<string, SimNode>,
): SimState {
  const cfg = { ...DEFAULT_SIM_CONFIG, ...config };
  const nodes = new Map<string, SimNode>();
  const centerX = width / 2;
  const centerY = height / 2;
  const spreadR = Math.min(width, height) * 0.3;

  for (const id of nodeIds) {
    const prev = existingNodes?.get(id);
    if (prev) {
      // Preserve old position with slight jitter
      nodes.set(id, {
        id,
        x: prev.x + (Math.random() - 0.5) * 20,
        y: prev.y + (Math.random() - 0.5) * 20,
        vx: 0,
        vy: 0,
        pinned: false,
        radius: prev.radius,
      });
    } else {
      // Random within circular area
      const angle = Math.random() * 2 * Math.PI;
      const r = Math.sqrt(Math.random()) * spreadR;
      nodes.set(id, {
        id,
        x: centerX + r * Math.cos(angle),
        y: centerY + r * Math.sin(angle),
        vx: 0,
        vy: 0,
        pinned: false,
        radius: 12,
      });
    }
  }

  return {
    nodes,
    edges: edges.map(([source, target]) => ({ source, target })),
    alpha: 1.0,
    frozen: false,
    convergenceFrames: 0,
    config: cfg,
  };
}

export function tick(state: SimState, width: number, height: number): void {
  if (state.frozen) return;

  const { nodes, edges, config } = state;
  const nodeArr = Array.from(nodes.values());
  const n = nodeArr.length;
  if (n === 0) return;

  const centerX = width / 2;
  const centerY = height / 2;
  const boundaryRadius = Math.min(width, height) * BOUNDARY_RADIUS_RATIO;
  const dragId = Array.from(nodes.entries()).find(([, v]) => v.pinned)?.[0] || null;

  const alpha = state.alpha;
  const repelK = config.repulsion * alpha;
  const springK = config.attraction * alpha;
  const centerK = config.centering * alpha;
  // Hermes-desktop damping: 0.5 + (1 - alpha) * 0.45 → 0.5~0.95 as alpha decays
  const damping = 0.5 + (1 - alpha) * 0.45;
  const maxSpeed = 10 + alpha * 30;

  // 1. Repulsion + collision: Coulomb-like pairwise O(n²)
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      const a = nodeArr[i];
      const b = nodeArr[j];
      let dx = b.x - a.x;
      let dy = b.y - a.y;
      let dist = Math.sqrt(dx * dx + dy * dy) || 1;
      if (dist < 1) { dist = 1; dx = 1; dy = 0; }

      const minDist = a.radius + b.radius + 10;
      const rf = repelK / (dist * dist);
      const fxA = (dx / dist) * rf;
      const fyA = (dy / dist) * rf;
      if (!a.pinned) { a.vx -= fxA; a.vy -= fyA; }
      if (!b.pinned) { b.vx += fxA; b.vy += fyA; }

      // Collision overlap correction
      if (dist < minDist) {
        const overlap = minDist - dist;
        const cf = overlap * 0.6;
        if (!a.pinned) { a.x -= (dx / dist) * cf; a.y -= (dy / dist) * cf; }
        if (!b.pinned) { b.x += (dx / dist) * cf; b.y += (dy / dist) * cf; }
      }
    }
  }

  // 2. Edge springs: Hooke-law
  const idealEdgeLen = config.linkDistance;
  for (const edge of edges) {
    const sa = nodes.get(edge.source);
    const sb = nodes.get(edge.target);
    if (!sa || !sb) continue;
    let dx = sb.x - sa.x;
    let dy = sb.y - sa.y;
    let dist = Math.sqrt(dx * dx + dy * dy) || 1;
    if (dist < 1) { dist = 1; dx = 1; dy = 0; }
    const displacement = dist - idealEdgeLen;
    let sf = displacement * springK;
    // Boost spring force when one end is being dragged
    if (dragId && (edge.source === dragId || edge.target === dragId)) {
      sf = displacement * (springK * config.dragForce);
    }
    if (!sa.pinned) { sa.vx += (dx / dist) * sf; sa.vy += (dy / dist) * sf; }
    if (!sb.pinned) { sb.vx -= (dx / dist) * sf; sb.vy -= (dy / dist) * sf; }
  }

  // 3. Apply forces + centering + velocity cap + damping + boundary
  let maxNodeSpeed = 0;
  for (const node of nodeArr) {
    if (node.pinned) continue;
    // Centering force
    node.vx += (centerX - node.x) * centerK;
    node.vy += (centerY - node.y) * centerK;
    // Speed cap
    const speed = Math.sqrt(node.vx * node.vx + node.vy * node.vy);
    if (speed > maxSpeed) {
      node.vx = (node.vx / speed) * maxSpeed;
      node.vy = (node.vy / speed) * maxSpeed;
    }
    // Apply velocity + damping
    node.x += node.vx;
    node.y += node.vy;
    node.vx *= damping;
    node.vy *= damping;

    // No hard boundary — centering force controls overall spread

    const sp = Math.sqrt(node.vx * node.vx + node.vy * node.vy);
    if (sp > maxNodeSpeed) maxNodeSpeed = sp;
  }

  // 4. Alpha decay (hermes-desktop: alpha *= 0.96 each frame)
  if (dragId) {
    // Gradual ramp-up — no instant jump that causes visual flash
    if (state.alpha < 0.01) state.alpha = 0.01; // seed if frozen
    if (state.alpha < 0.15) state.alpha += 0.02; // ramp up ~8 frames
    state.alpha *= 0.98;
    state.frozen = false;
    state.convergenceFrames = 0;
  } else if (!state.frozen) {
    state.alpha *= ALPHA_DECAY_FACTOR;
  }

  // 5. Convergence detection
  if (!dragId) {
    if (state.alpha <= ALPHA_MIN) {
      // Alpha decayed to minimum — freeze immediately
      state.frozen = true;
      state.alpha = 0;
    } else if (maxNodeSpeed < CONVERGENCE_SPEED_THRESHOLD && state.alpha < CONVERGENCE_ALPHA_THRESHOLD) {
      state.convergenceFrames++;
      if (state.convergenceFrames >= CONVERGENCE_FRAMES_NEEDED) {
        state.frozen = true;
        state.alpha = 0;
      }
    } else if (maxNodeSpeed >= CONVERGENCE_SPEED_THRESHOLD) {
      state.convergenceFrames = 0;
    }
  }
}

// Pin a node at a position (for drag). Keeps alpha ≥ 0.3.
export function pinNode(state: SimState, id: string, x: number, y: number): void {
  const node = state.nodes.get(id);
  if (!node) return;
  node.pinned = true;
  node.x = x;
  node.y = y;
  node.vx = 0;
  node.vy = 0;
  state.frozen = false;       // unfreeze; alpha rises gradually in tick
  state.convergenceFrames = 0;
}

// Move a pinned node during drag
export function movePinned(state: SimState, id: string, x: number, y: number): void {
  const node = state.nodes.get(id);
  if (!node || !node.pinned) return;
  node.x = x;
  node.y = y;
}

// Release a pinned node
export function unpinNode(state: SimState, id: string): void {
  const node = state.nodes.get(id);
  if (!node) return;
  node.pinned = false;
}

export function setConfig(state: SimState, partial: Partial<SimConfig>): void {
  Object.assign(state.config, partial);
}
