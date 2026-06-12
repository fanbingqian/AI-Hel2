# 2D 知识图谱渲染优化方案

> 基于 Obsidian 图谱源码逆向分析 + AI-Hel2 现有实现对比  
> 日期: 2026-06-12

## 一、技术对比

| 维度 | Obsidian | AI-Hel2 (当前) |
|------|----------|---------------|
| 图形 API | WebGL（Pixi.js）GPU 渲染 | SVG（d3.js）CPU 渲染 |
| 力计算 | Web Worker 异步，不阻塞 UI | 主线程 `requestAnimationFrame`，O(n²) |
| 节点图形 | `PIXI.Graphics` 圆 + `tint` 着色 | `<circle>` + `fill` |
| 文字 | `PIXI.Text`，跟随缩放动态淡出 | `<text>`，固定字号 5px，#aaa |
| 连线 | `PIXI.Sprite` 纹理拉伸 + `Graphics` 箭头 | `<path>` SVG stroke |
| 缩放范围 | 1/128 ~ 8×（指数平滑插值） | 0.2 ~ 3×（d3.zoom 线性） |
| 节点大小 | `clamp(3√(weight+1), 8, 30)` × multiplier | `radius = 12` 固定值 |
| 大图性能 | GPU 批量，万级节点流畅 | SVG DOM = 瓶颈，千级卡顿 |

### Obsidian 渲染管线

```
Vault 文件 → Metadata Cache Worker 索引 → 节点+边列表
  → graph.json 用户设置
  → Web Worker 力导向计算（每 tick: 中心引力/斥力/弹簧力/alpha 衰减）
  → postMessage 回主线程
  → Pixi.js WebGL 渲染循环（requestAnimationFrame）
    - hanger 容器: 平移 + 缩放
    - 节点: circle.position/alpha/tint/scale + text.position/alpha
    - 边: sprite 拉伸 + arrow 旋转
  → 用户交互（拖拽/缩放/点击导航/悬停高亮）
```

### 我们的渲染管线

```
entities + relations → buildGraphData → FGNode[] + FGLink[]
  → createSimulation（主线程）
  → requestAnimationFrame loop:
    tick() → Coulomb O(n²) 斥力 + Hooke 弹簧 + 碰撞检测 + 向心力
    D3 update: circle.cx/cy + text.x/y + path.d
```

---

## 二、核心问题：节点多时「发丝球」（Hairball Graph）

当实体数 > 200 时，满屏节点和连线互相重叠，完全无法辨认。Obsidian 通过**缩放联动**缓解此问题，但未根本解决。以下方案按优先级排列。

### P0 — 缩放联动淡出（~30 行改动）

借鉴 Obsidian 的 scale-based 信息密度控制：

```
缩小时 (scale < 0.5):
  - 文字 opacity → 0（只看结构）
  - 低度节点 radius → 3px（去噪）
  - 连线 opacity → 0.05（只看骨架）

放大时 (scale > 1.5):
  - 文字完全显示
  - 所有节点正常大小
  - 连线正常
```

Obsidian 公式可直接复用：
```
nodeScale = sqrt(1 / scale)
textAlpha = clamp(log2(scale) + 1, 0, 1)
```

效果：远看整体拓扑，近看局部细节，无需任何额外 UI。

### P1 — 节点大小基于连接权重（~10 行改动）

当前所有节点 `radius = 12` 一样大。改为：

```
radius = clamp(4 + degree * 0.5, 5, 25) * nodeSizeMultiplier
```

核心枢纽实体（连接数 > 20）大而显眼，边缘实体（连接数 1-2）小而低调。`degMap` 已在 `graphAdapter.ts` 中计算好，直接取用。

### P1 — 最小连接数过滤（~20 行改动）

在配置面板「筛选」区增加 `minDegree` 滑块：

```
值 = 0 → 显示所有节点
值 = 2 → 只显示 degree >= 2
值 = 5 → 只显示核心枢纽
```

逻辑上就是 `showOrphans`（degree = 0 过滤）的泛化版本。`buildGraphData` 已支持 `showOrphans`，扩展为 `minDegree` 即可。

### P2 — 悬停聚焦增强（~15 行改动）

当前已实现 hover 高亮，但连线 dimming 不够狠。增强：

```
hover 目标节点:
  - 目标节点 + 邻居: opacity 1, stroke = HOVER_RING
  - 其他节点: opacity 0.08
  - 目标相关连线: opacity 0.7, stroke = EDGE_HOVER
  - 其他连线: opacity 0.02 (当前 EDGE_DIM 值是 0.06，仍然可见)
```

### P2 — 实体类型多选过滤（~40 行改动）

在筛选区增加「实体类型」多选复选框：

```
☑ 人物  ☑ 概念  ☐ 文件  ☐ 事件  ☑ 组织  ☐ 标签
```

`buildGraphData` 已有 `entityType` 字段，加一个 `Set<string>` 过滤参数即可。

### P3 — 社区折叠 / 聚合视图（~100 行改动）

将紧密连接的子图折叠为「社区节点」：

```
折叠态: 一个大社区节点，label = "12 entities", 颜色区分
展开态: 双击进入子图谱（递归支持）
```

依赖：`nexusRunCommunity()` 已生成社区划分数据，可直接用于聚合。

### P3 — Web Worker 力计算（~150 行改动）

将 `tick()` 移入 Web Worker，主线程只做渲染。当节点 > 500 时 O(n²) 计算不再阻塞 UI。

```
主线程:               Worker:
  postMessage({         onmessage:
    nodes, edges,         tick() → 更新位置
    config                postMessage(positions)
  })                   }
  → RAF 纯渲染
```

### P3 — Pixi.js WebGL 迁移（架构级改动）

从 SVG DOM 迁移到 WebGL 渲染管线，性能从千级提升到万级。Obsidian 级别的渲染质量。需引入 `pixi.js` 依赖，重写 `ForceGraph2DWrapper`。

---

## 三、Obsidian 可借鉴的设计模式

| 模式 | 说明 | 我们现状 |
|------|------|---------|
| **hanger 容器** | 所有元素挂在一个 Container 下，平移/缩放只需操作这一个对象 | 用 d3.zoom + g transform，效果等同 |
| **tint 着色** | 圆用白色填充 + tint 叠加颜色，不用重新绘制 | SVG fill 直接设值，改动更简单 |
| **鼠标位置保持缩放** | 缩放时计算 `worldPos = (mouse - pan) / oldScale`，维持鼠标所指位置不变 | d3.zoom 自带此行为 ✅ |
| **模拟退火冻结** | 连续 N 帧速度低于阈值 → `frozen = true`，停止计算节省 CPU | 已实现 `CONVERGENCE_FRAMES_NEEDED = 8` ✅ |
| **节点位置保留** | 数据刷新时保留旧位置 + 微小抖动，避免图谱全盘重排 | 已实现 `existingNodes` -> `prev` 保留 ✅ |

---

## 四、实施路线

```
Phase 1 (即时)     P0 缩放联动淡出 + P1 节点权重半径 + P1 最小连接数
                   预期: <200 节点时体验接近 Obsidian

Phase 2 (短期)     P2 悬停聚焦增强 + P2 实体类型多选
                   预期: 可高效探索 200-500 节点图谱

Phase 3 (中期)     P3 社区折叠 + P3 Web Worker
                   预期: 500-2000 节点可流畅交互

Phase 4 (远期)     P3 Pixi.js 迁移
                   预期: 万级节点，达到 Obsidian 级别渲染性能
```
