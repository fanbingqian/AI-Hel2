import { useEffect, useRef, useCallback, useState, useMemo } from "react";
import ForceGraph3D from "react-force-graph-3d";
import * as THREE from "three";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { useChatStore } from "../../stores/chatStore";
import { buildGraphData, type FGNode, type FGLink } from "./graphAdapter";
import { NodeContextMenu } from "./NodeContextMenu";
import styles from "./KnowledgeSphere.module.css";

const SPHERE_SHELL_R = 32;
const NODE_REL_SIZE = 7;

function createSphereShell(): THREE.Group {
  const group = new THREE.Group();
  const wireGeo = new THREE.IcosahedronGeometry(SPHERE_SHELL_R, 5);
  const wireMesh = new THREE.Mesh(
    wireGeo,
    new THREE.MeshBasicMaterial({ color: "#1a2a1a", wireframe: true, transparent: true, opacity: 0.1, depthWrite: false }),
  );
  group.add(wireMesh);
  const shellGeo = new THREE.SphereGeometry(SPHERE_SHELL_R - 0.08, 64, 64);
  const shellMat = new THREE.ShaderMaterial({
    uniforms: { uTime: { value: 0 } },
    vertexShader: `
      varying vec3 vNormal; varying vec3 vPos;
      void main() {
        vec4 wp = modelMatrix * vec4(position, 1.0);
        vPos = wp.xyz;
        vNormal = normalize(mat3(modelMatrix) * normal);
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: `
      varying vec3 vNormal; varying vec3 vPos;
      uniform float uTime;
      void main() {
        vec3 V = normalize(cameraPosition - vPos);
        float f = 1.0 - abs(dot(V, vNormal));
        f = pow(f, 4.0);
        float a = 0.02 + f * 0.05;
        gl_FragColor = vec4(0.03, 0.06, 0.03, a);
      }
    `,
    transparent: true,
    depthWrite: false,
  });
  group.add(new THREE.Mesh(shellGeo, shellMat));
  return group;
}

// Simple 2D sprite — just a filled circle, no rings or glow
function createNodeSprite(node: FGNode): THREE.Sprite {
  const r = node._sphereRadius;
  const size = 64;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  ctx.beginPath();
  ctx.arc(size / 2, size / 2, size * 0.45, 0, Math.PI * 2);
  ctx.fillStyle = node.color;
  ctx.fill();

  const texture = new THREE.CanvasTexture(canvas);
  texture.minFilter = THREE.LinearFilter;
  const mat = new THREE.SpriteMaterial({
    map: texture,
    transparent: true,
    opacity: 1.0,
    depthWrite: false,
    depthTest: true,
  });
  const sprite = new THREE.Sprite(mat);
  const scale = r * 1.5;
  sprite.scale.set(scale, scale, 1);
  return sprite;
}

export function ForceGraph3DWrapper() {
  const fgRef = useRef<any>(null);
  const shellGroupRef = useRef<THREE.Group | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [dims, setDims] = useState({ width: 800, height: 600 });

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) {
        const { width, height } = e.contentRect;
        setDims({ width, height });
      }
    });
    ro.observe(el);
    setDims({ width: el.clientWidth, height: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  const entities = useKnowledgeStore((s) => s.entities);
  const relations = useKnowledgeStore((s) => s.relations);
  const selectedEntityId = useKnowledgeStore((s) => s.selectedEntityId);
  const selectEntity = useKnowledgeStore((s) => s.selectEntity);
  const focusedNodeId = useKnowledgeStore((s) => s.focusedNodeId);
  const focusDepth = useKnowledgeStore((s) => s.focusDepth);
  const setFocusedNode = useKnowledgeStore((s) => s.setFocusedNode);
  const setFocusDepth = useKnowledgeStore((s) => s.setFocusDepth);
  const showOrphans = useKnowledgeStore((s) => s.showOrphans);
  const showFiles = useKnowledgeStore((s) => s.showFiles);
  const referenceToChat = useKnowledgeStore((s) => s.referenceToChat);
  const inferences = useKnowledgeStore((s) => s.inferences);
  const showInferenceEdges = useKnowledgeStore((s) => s.showInferenceEdges);
  const graphSettings = useKnowledgeStore((s) => s.graphSettings3D);
  const sendMessage = useChatStore((s) => s.sendMessage);

  const hoveredIdRef = useRef<string | null>(null);
  const nodeMaterialsRef = useRef<Map<string, THREE.SpriteMaterial>>(new Map());
  const [isHovering, setIsHovering] = useState(false);

  const [contextMenu, setContextMenu] = useState<{
    entityId: string;
    entityName: string;
    x: number;
    y: number;
  } | null>(null);

  const graphData = useMemo(() => {
    return buildGraphData(entities, relations, inferences, {
      focusedNodeId,
      focusDepth,
      showOrphans,
      showFiles,
      showInferenceEdges,
      nodeRelSize: NODE_REL_SIZE,
    });
  }, [entities, relations, inferences, focusedNodeId, focusDepth, showOrphans, showFiles, showInferenceEdges]);

  const allLinks = useMemo(() => {
    return [...graphData.links, ...graphData.infLinks];
  }, [graphData.links, graphData.infLinks]);

  const nodesRef = useRef(graphData.nodes);
  nodesRef.current = graphData.nodes;

  // Init: sphere shell + camera — once, after ForceGraph3D is ready
  useEffect(() => {
    let done = false;
    const tryInit = () => {
      const fg = fgRef.current;
      if (!fg || done) return;
      try {
        const scene = fg.scene();
        if (!scene) return;
        if (!shellGroupRef.current) {
          shellGroupRef.current = createSphereShell();
          scene.add(shellGroupRef.current);
        }
        fg.cameraPosition({ z: 80 });
        done = true;
      } catch { /* ignore */ }
    };
    // Poll until ForceGraph3D ref is ready, then init once
    const iv = setInterval(() => {
      tryInit();
      if (done) {
        clearInterval(iv);
        // Schedule zoomToFit after data has loaded and warmup done
        setTimeout(() => {
          try {
            if (nodesRef.current.length > 0) {
              fgRef.current?.zoomToFit(600, 80);
            }
          } catch { /* ignore */ }
        }, 600);
      }
    }, 300);
    return () => clearInterval(iv);
  }, []);

  // Zoom to fit + reheat when data first arrives or node count changes
  const prevLen = useRef(0);
  useEffect(() => {
    const len = graphData.nodes.length;
    if (len > 0 && prevLen.current === 0) {
      setTimeout(() => {
        try {
          fgRef.current?.zoomToFit(600, 80);
        } catch { /* ignore */ }
      }, 600);
    }
    prevLen.current = len;
  }, [graphData.nodes.length]);

  // Physics — mild settings that allow natural expansion without NaN
  useEffect(() => {
    const fg = fgRef.current;
    if (!fg) return;
    try {
      fg.d3Force("charge")?.strength(-graphSettings.repulsion * 0.0005);
      fg.d3Force("link")?.distance(graphSettings.linkDistance * 0.5)?.strength(graphSettings.attraction * 0.3);
      fg.d3Force("center")?.strength(graphSettings.centering * 200);
    } catch { /* ignore */ }
  }, [graphSettings]);

  // Sprite opacity sync for hover dimming
  const syncSpriteVisuals = useCallback((hoveredId: string | null, selectedId: string | null) => {
    const materials = nodeMaterialsRef.current;
    if (materials.size === 0) return;
    const links = allLinks;
    materials.forEach((mat, id) => {
      if (id === selectedId) { mat.opacity = 1; return; }
      if (!hoveredId) { mat.opacity = 1; return; }
      if (id === hoveredId) { mat.opacity = 1; return; }
      const isRelated = links.some(
        (l) =>
          (l.source === hoveredId && l.target === id) ||
          (l.target === hoveredId && l.source === id) ||
          (typeof l.source === "object" && (l.source as any)?.id === hoveredId && (l.target as any)?.id === id) ||
          (typeof l.target === "object" && (l.target as any)?.id === hoveredId && (l.source as any)?.id === id),
      );
      mat.opacity = isRelated ? 0.65 : 0.18;
    });
  }, [allLinks]);

  useEffect(() => {
    syncSpriteVisuals(hoveredIdRef.current, selectedEntityId);
  }, [selectedEntityId, syncSpriteVisuals]);

  const nodeThreeObject = useCallback((node: FGNode) => {
    const sprite = createNodeSprite(node);
    nodeMaterialsRef.current.set(node.id, sprite.material as THREE.SpriteMaterial);
    return sprite;
  }, []);

  const linkColor = useCallback(
    (link: FGLink) => {
      const a = graphSettings.edgeOpacity;
      if (link.relationTypes.includes("__inference__")) return `rgba(255,255,255,${a * 0.7})`;
      return `rgba(255,255,255,${a})`;
    },
    [graphSettings.edgeOpacity],
  );

  const linkWidth = useCallback(
    (link: FGLink) => {
      if (link.relationTypes.includes("__inference__")) return 1.2;
      if (link.merged) return 2.0;
      return 1.0;
    },
    [],
  );

  const linkDirectionalParticles = useCallback(
    (link: FGLink) => link.relationTypes.includes("__inference__") ? 2 : 0,
    [],
  );

  const linkDirectionalParticleSpeed = useCallback(
    (link: FGLink) => (link.relationTypes.includes("__inference__") ? 0.005 : 0.01),
    [],
  );

  const handleNodeClick = useCallback(
    (node: FGNode) => { selectEntity(node.id); },
    [selectEntity],
  );

  const handleNodeRightClick = useCallback(
    (node: FGNode, event: MouseEvent) => {
      setContextMenu({ entityId: node.id, entityName: node.name, x: event.clientX, y: event.clientY });
    },
    [],
  );

  const handleNodeHover = useCallback((node: FGNode | null) => {
    const hoveredId = node?.id ?? null;
    hoveredIdRef.current = hoveredId;
    setIsHovering(!!hoveredId);
    syncSpriteVisuals(hoveredId, selectedEntityId);
  }, [syncSpriteVisuals, selectedEntityId]);

  const handleBackgroundClick = useCallback(() => {
    selectEntity("");
  }, [selectEntity]);

  const nodeLabel = useCallback((node: FGNode) => {
    if (!graphSettings.showLabels) return "";
    return node.name.length > 10 ? node.name.slice(0, 9) + "…" : node.name;
  }, [graphSettings.showLabels]);

  const focusEntityName = useMemo(() => {
    if (!focusedNodeId) return null;
    return entities.find((e) => e.id === focusedNodeId)?.name ?? focusedNodeId;
  }, [focusedNodeId, entities]);

  return (
    <div ref={containerRef} className={styles.container}>
      <ForceGraph3D
        ref={fgRef}
        width={dims.width}
        height={dims.height}
        graphData={{ nodes: graphData.nodes, links: allLinks }}
        nodeId="id"
        nodeVal="val"
        nodeColor="color"
        nodeLabel={nodeLabel as any}
        nodeThreeObject={nodeThreeObject}
        nodeThreeObjectExtend={false}
        nodeRelSize={NODE_REL_SIZE * graphSettings.nodeScale}
        linkColor={linkColor}
        linkWidth={linkWidth}
        linkDirectionalParticles={linkDirectionalParticles}
        linkDirectionalParticleSpeed={linkDirectionalParticleSpeed}
        linkDirectionalParticleWidth={0.4}
        linkDirectionalParticleColor={() => "#F59E0B"}
        onNodeClick={handleNodeClick}
        onNodeRightClick={handleNodeRightClick}
        onNodeHover={handleNodeHover}
        onBackgroundClick={handleBackgroundClick}
        backgroundColor="#1A1A1A"
        enableNodeDrag={true}
        enableNavigationControls={true}
        controlType="orbit"
        warmupTicks={300}
        d3VelocityDecay={isHovering ? 0.85 : 0.35}
        d3AlphaDecay={0.02}
        showNavInfo={false}
      />

      <button
        type="button"
        className={styles.fitButton}
        title="适配窗口"
        onClick={() => {
          try { fgRef.current?.zoomToFit(400, 50); } catch { /* ignore */ }
        }}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
          <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" />
        </svg>
      </button>

      {focusedNodeId && (
        <div className={styles.focusLabel}>
          <span>已聚焦: {focusEntityName}</span>
          <span className={styles.depthSelector}>
            {[1, 2, 3].map((d) => (
              <button
                key={d}
                type="button"
                className={focusDepth === d ? styles.depthActive : ""}
                onClick={() => setFocusDepth(d)}
                title={`${d} 跳邻居`}
              >
                {d}
              </button>
            ))}
          </span>
          <button type="button" onClick={() => setFocusedNode(null)}>退出</button>
        </div>
      )}

      {contextMenu && (
        <NodeContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          entityName={contextMenu.entityName}
          onDetail={() => {
            selectEntity(contextMenu.entityId);
            setContextMenu(null);
          }}
          onReference={async () => {
            try {
              const ref = await referenceToChat(contextMenu.entityId);
              sendMessage(`请根据以下知识库实体回答：${ref.markdown_ref}\n\n摘要：${ref.summary}`);
            } catch (e) {
              console.error("Reference failed:", e);
            }
            setContextMenu(null);
          }}
          onFocus={() => {
            setFocusedNode(contextMenu.entityId);
            selectEntity(contextMenu.entityId);
            setContextMenu(null);
          }}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  );
}
