# Phase 4: Pixi.js WebGL 迁移计划

> 从 SVG/d3.js 迁移到 WebGL/Pixi.js，达到 Obsidian 级别的图谱渲染性能  
> 日期: 2026-06-12 | 预估工期: 3-5 天

## 一、目标与收益

| 维度 | 当前 (SVG/d3) | 目标 (Pixi.js WebGL) |
|------|--------------|---------------------|
| 渲染管线 | CPU 绑定 DOM 操作 | GPU 批量绘制 |
| 节点上限 | ~500 节点流畅 | ~10,000 节点流畅 |
| 力计算 | 主线程 RAF | Web Worker 并行 |
| 文字渲染 | SVG `<text>` 5px | PIXI.Text + SDF 字体 |
| 缩放范围 | 0.2 ~ 3× | 1/128 ~ 8× |
| 内存占用 | DOM 节点数 × ~2KB | GPU buffer 共享 |
| 首帧时间 | O(n) DOM 创建 | O(1) buffer 上传 |

## 二、技术架构

```
┌─ React Component Layer ────────────────────────────────┐
│  ForceGraph2DWrapper.tsx                                │
│  - 状态管理: knowledgeStore (不变)                       │
│  - 配置面板: GraphSettingsPanel (不变)                   │
│  - 数据适配: graphAdapter.ts (不变)                      │
│  - PixiApp 容器 ref (新增)                              │
└────────────────────┬───────────────────────────────────┘
                     │
┌─ Pixi Application Layer ───────────────────────────────┐
│  pixiGraph.ts (新文件)                                  │
│  - PIXI.Application 初始化 (WebGL context)              │
│  - hanger Container → 平移/缩放                        │
│  - Node Layer → PIXI.Graphics 圆形                     │
│  - Edge Layer → PIXI.Sprite 拉伸                       │
│  - Label Layer → PIXI.Text + BitmapFont                 │
│  - Hover/Click 交互 → Federated Events                 │
└────────────────────┬───────────────────────────────────┘
                     │ postMessage
┌─ Web Worker Layer ─────────────────────────────────────┐
│  physics.worker.ts (新文件)                             │
│  - 从 physics.ts 移植 tick()                            │
│  - 每 ~16ms 回传 { positions[], frozen }                │
│  - 主线程只做渲染，不参与计算                            │
└────────────────────────────────────────────────────────┘
```

## 三、文件变更清单

### 3.1 新增文件

| 文件 | 职责 | 预估行数 |
|------|------|---------|
| `src/components/sphere/pixiGraph.ts` | Pixi.js 图谱渲染核心 | ~300 |
| `src/components/sphere/physics.worker.ts` | Web Worker 力计算 | ~80 |
| `src/components/sphere/PixiGraphWrapper.tsx` | React ↔ Pixi 桥接组件 | ~100 |

### 3.2 修改文件

| 文件 | 改动 | 说明 |
|------|------|------|
| `ForceGraph2DWrapper.tsx` | 新增渲染模式切换 | SVG / WebGL 二选一 |
| `GraphSettingsPanel.tsx` | 新增 WebGL 开关 | `useWebGL: boolean` |
| `knowledge.ts` | 新增配置项 | `useWebGL: boolean` |
| `package.json` | 新增依赖 | `pixi.js@^8` |

### 3.3 保留不变

| 模块 | 原因 |
|------|------|
| `graphAdapter.ts` | 数据转换逻辑与渲染无关 |
| `knowledgeStore.ts` | 状态管理不变 |
| `physics.ts` | 力算法逻辑不变，仅迁移到 Worker |
| `GraphSettingsPanel.tsx` 力度/外观/筛选 | 配置接口不变 |
| 图例/详情面板 | 纯 React DOM，不受影响 |

## 四、核心实现步骤

### Step 1: 环境准备 (0.5 天)

```
- npm install pixi.js@^8
- 创建 pixiGraph.ts 骨架
- 创建 physics.worker.ts（从 physics.ts 提取）
- Vite 配置 worker 打包
```

**Pixi.js v8 关键 API：**
```typescript
import { Application, Graphics, Text, TextStyle, Container, Sprite, Texture } from 'pixi.js';

const app = new Application();
await app.init({ background: 'transparent', resizeTo: container, antialias: true });
container.appendChild(app.canvas);
```

### Step 2: PixiGraph 核心渲染 (1.5 天)

```typescript
// pixiGraph.ts — 对标 Obsidian 的场景层级
class PixiGraph {
  app: Application;
  hanger: Container;        // 平移/缩放根容器
  nodeLayer: Container;     // 节点层
  edgeLayer: Container;     // 连线层
  labelLayer: Container;    // 文字层
  
  // 节点缓存：复用 PIXI.Graphics 对象
  nodeMap: Map<string, Graphics>;
  labelMap: Map<string, Text>;
  edgeMap: Map<string, Sprite>;
  
  // 配置
  config: GraphRenderConfig;
  
  // 核心方法
  updateData(nodes: FGNode[], links: FGLink[]): void;
  updatePositions(positions: Map<string, {x,y}>): void;
  setTransform(panX: number, panY: number, scale: number): void;
  highlightNode(nodeId: string | null): void;
  selectNode(nodeId: string | null): void;
  dispose(): void;
}
```

**节点渲染（对标 Obsidian）：**
```typescript
createNode(n: FGNode): Graphics {
  const g = new Graphics();
  // 白色填充 + tint 着色（避免重新绘制）
  g.circle(0, 0, n._sphereRadius)
   .fill({ color: 0xFFFFFF })
   .stroke({ color: 0x000000, width: 1, alpha: 0.3 });
  g.tint = hexToColor(n._color);  // 颜色叠加
  g.eventMode = 'static';
  g.cursor = 'pointer';
  g.label = n.id;  // 用于交互识别
  return g;
}
```

**连线渲染（对标 Obsidian 的 Sprite 拉伸）：**
```typescript
createEdge(source: {x,y}, target: {x,y}): Sprite {
  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const length = Math.sqrt(dx*dx + dy*dy);
  const angle = Math.atan2(dy, dx);
  
  const sprite = new Sprite(Texture.WHITE);
  sprite.width = length;
  sprite.height = 0.5;
  sprite.anchor.set(0, 0.5);
  sprite.position.set(source.x, source.y);
  sprite.rotation = angle;
  sprite.tint = 0x9999aa;
  return sprite;
}
```

**文字渲染（对标 Obsidian 的 PIXI.Text）：**
```typescript
createLabel(n: FGNode): Text {
  const style = new TextStyle({
    fontSize: 12,
    fill: 0xaaaaaa,
    fontFamily: 'Microsoft YaHei, sans-serif',
    align: 'center',
  });
  const text = new Text({ 
    text: n.name.slice(0, 12), 
    style 
  });
  text.anchor.set(0.5, 0);
  text.resolution = 2; // 2x 清晰度
  return text;
}
```

### Step 3: 交互实现 (0.5 天)

```typescript
// 缩放 — 对标 Obsidian 的指数平滑插值
handleWheel(e: WheelEvent) {
  const direction = e.deltaY > 0 ? -1 : 1;
  this.targetScale *= direction > 0 ? 1.1 : 0.9;
  this.targetScale = Math.max(1/128, Math.min(8, this.targetScale));
  // 缩放时保持鼠标位置不变
  const worldPos = this.screenToWorld(e.clientX, e.clientY);
  this.panX -= worldPos.x * (newScale - this.scale);
  this.panY -= worldPos.y * (newScale - this.scale);
}

// 每帧平滑缩放
updateZoom() {
  const diff = this.targetScale / this.scale;
  if (Math.abs(diff - 1) < 0.001) return;
  this.scale = this.scale + (this.targetScale - this.scale) * 0.85; // 指数平滑
  this.hanger.scale = this.scale;
  // 缩放联动：节点大小 + 文字透明度
  this.nodeScale = Math.sqrt(1 / this.scale);
  this.textAlpha = Math.max(0, Math.min(1, Math.log2(this.scale) + 1));
}

// 拖拽
handleDragStart(nodeId: string) { pin node; }
handleDragMove(nodeId: string, x: number, y: number) { update position; }
handleDragEnd(nodeId: string) { unpin node; }

// 悬停高亮
handleHover(nodeId: string | null) {
  if (!nodeId) { reset all alpha to 1; return; }
  const related = getNeighbors(nodeId);
  for (const [id, g] of this.nodeMap) {
    g.alpha = id === nodeId || related.has(id) ? 1 : 0.08;
  }
  for (const [key, s] of this.edgeMap) {
    s.alpha = key.includes(nodeId) ? 1 : 0.02;
  }
}
```

### Step 4: React 桥接 (0.5 天)

```typescript
// PixiGraphWrapper.tsx
export function PixiGraphWrapper() {
  const containerRef = useRef<HTMLDivElement>(null);
  const pixiRef = useRef<PixiGraph | null>(null);
  
  // Store data
  const entities = useKnowledgeStore(s => s.entities);
  const relations = useKnowledgeStore(s => s.relations);
  const graphSettings = useKnowledgeStore(s => s.graphSettings2D);
  // ... same selectors as ForceGraph2DWrapper
  
  // Init Pixi
  useEffect(() => {
    if (!containerRef.current) return;
    const pg = new PixiGraph(containerRef.current);
    pixiRef.current = pg;
    return () => pg.dispose();
  }, []);
  
  // Data update → Pixi
  useEffect(() => {
    const pg = pixiRef.current;
    if (!pg) return;
    const { nodes, links } = buildGraphData(entities, relations, inferences, opts);
    pg.updateData(nodes, links);
  }, [entities, relations, graphSettings]);
  
  // Worker positions → Pixi
  useEffect(() => {
    // listen to worker messages, call pg.updatePositions()
  }, []);
  
  return <div ref={containerRef} style={{ width: '100%', height: '100%' }} />;
}
```

### Step 5: Web Worker 力计算 (0.5 天)

```typescript
// physics.worker.ts
import { tick, createSimulation } from './physics';

let sim: SimState;
let W = 800, H = 600;
let running = true;

self.onmessage = (e) => {
  const { type, ...data } = e.data;
  switch (type) {
    case 'init':
      sim = createSimulation(data.nodeIds, data.edges, data.config, W, H);
      break;
    case 'resize': W = data.width; H = data.height; break;
    case 'config': Object.assign(sim.config, data); sim.alpha = 1; break;
    case 'stop': running = false; break;
  }
};

function loop() {
  if (!sim || !running) { setTimeout(loop, 50); return; }
  if (!sim.frozen) tick(sim, W, H);
  // Post positions as plain object for transfer speed
  const positions: Record<string, {x:number,y:number}> = {};
  for (const [id, n] of sim.nodes) positions[id] = { x: n.x, y: n.y };
  self.postMessage({ positions, frozen: sim.frozen });
  setTimeout(loop, 16); // ~60fps physics
}
loop();
```

### Step 6: 双模式切换 (0.5 天)

在 `ForceGraph2DWrapper` 中根据 `graphSettings.useWebGL` 切换：

```typescript
export function ForceGraph2DWrapper() {
  const useWebGL = useKnowledgeStore(s => s.graphSettings2D.useWebGL);
  
  if (useWebGL) return <PixiGraphWrapper />;
  return <SVGGraph />; // 现有 SVG 实现
}
```

## 五、风险评估

| 风险 | 影响 | 缓解 |
|------|------|------|
| Pixi.js v8 API 不稳定 | 编译/运行时错误 | 锁定版本 `^8.2` |
| Web Worker 消息延迟 | 位置更新滞后 | 主线程保留 direct tick 回退 |
| 移动端 WebGL 兼容性 | 部分设备黑屏 | 检测 `WEBGL` context 失败 → fallback SVG |
| 社区折叠等自定义渲染 | 与 Pixi 原语冲突 | 社区节点用更大 Graphics + 虚线边框区分 |
| 内存泄漏 | 长时运行崩溃 | `dispose()` 清理所有 Texture/Graphics |

## 六、测试清单

- [ ] 500 / 1000 / 5000 节点性能基准（FPS + 内存）
- [ ] 缩放联动淡出（文字 + 节点 + 连线）
- [ ] 悬停高亮 + 点击导航
- [ ] 拖拽节点 + 力模拟恢复
- [ ] 社区折叠模式
- [ ] 颜色分组 + 类型过滤
- [ ] 配置面板实时联动（力度/外观/筛选）
- [ ] 窗口 resize → Pixi resize
- [ ] 移动端触控（pinch zoom + tap）
- [ ] SVG fallback 切换

## 七、里程碑

```
Day 1  ████  Step 1+2  环境 + 核心渲染（节点/边/文字）
Day 2  ████  Step 3    交互（缩放/拖拽/悬停）
Day 3  ████  Step 4+5  React 桥接 + Web Worker
Day 4  ████  Step 6    双模式切换 + 测试 + 性能调优
Day 5  ████  Buffer    社区折叠适配 + 移动端 + 文档
```
