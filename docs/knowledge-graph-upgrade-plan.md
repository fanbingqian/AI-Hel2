# AI-Hel 知识图谱架构升级方案

## Context

当前 AI-Hel2 的知识图谱使用 8 种硬编码实体类型和 8 种硬编码关系类型，LLM 提取虽有"自由描述"提示但前端只能渲染 8 种颜色。Focus 模式仅 1 跳邻居。文件与实体割裂——文件被提取后原始文件上下文丢失。属性系统 `properties` 字段始终为 `'{}'`。物理引擎 O(n²) 和 Canvas 2D 渲染在 500+ 节点时成为瓶颈。

本方案覆盖 7 项改进，按依赖关系分为 4 个阶段。当前 MD 文档渲染仅有一行颜色 CSS，h1-h6、段落间距、代码块、引用块均使用浏览器默认样式，阅读体验接近纯文本。

## 总体架构

```
改进后的数据模型:
┌─────────────────────────────────────────────┐
│              同一知识图谱                     │
│  ┌──────────┐    ┌──────────┐               │
│  │ 文件节点  │    │ 实体节点  │               │
│  │ (灰色圆)  │    │ (自由类型+自动颜色)│       │
│  └─────┬─────┘    └─────┬─────┘             │
│        │ contains       │ depends_on         │
│        │                │ created_by         │
│        │    ┌───────────┤ uses               │
│        └────┤  LLM 语义边 │ ... (自由输出)     │
│             └───────────┤                    │
│        ┌───────────────┤ [[wikilink]]        │
│        │ 文件间链接边    │                    │
│        └───────────────┘                    │
└─────────────────────────────────────────────┘

三种语义角色，同一种圆形:
  ┌─ 实体节点: entity_type = LLM自由输出, 颜色=色相环hash分配
  ├─ 文件节点: entity_type = "__file__", 颜色=浅灰 #9CA3AF
  └─ 孤岛节点: degree = 0, 虚线边框 + 低透明度

渲染层:
  2D: PixiJS (WebGL) + Barnes-Hut 优化物理 → 支持 2000+ 节点
  3D: Three.js (R3F) + 3D 物理 → 保持不变
```

---

## 阶段一：类型系统解耦 + 多跳 + 孤岛 + 属性（无渲染改动）

### Step 1: 实体类型从硬编码枚举改为自由字符串 + 自动聚类

**后端 (Rust):**

- `models/knowledge.rs`: 删除 `EntityType` 和 `RelationType` 枚举，改为 `pub type EntityType = String`
- `knowledge_service.rs`:
  - 删除 `parse_entity_type()` 和 `parse_relation_type()` 的枚举映射，直接透传字符串
  - `extract_entities_local()` 中回退正则保持类型建议，但改为自由字符串
  - `write_extraction_result()` 已是自由字符串，无需改动
  - `get_smart_display()` 中类型计数改为动态 Map
- **`nexus_analyze_types()`** 增强:
  - 已有 Levenshtein 相似度聚类，增强自动发现稀有类型并建议合并
  - 返回聚类建议时附带代表性实体名称列表
  - 新增 Tauri command: `nexus_list_types` → 返回所有类型及实体数量
  - 新增 Tauri command: `nexus_merge_types` → 用户确认合并

**前端:**

- `types/knowledge.ts`: 删除 `ENTITY_TYPE_COLORS` 硬编码映射
- `knowledgeStore.ts`: 新增:
  - `typeColors: Record<string, string>` — 自动从色相环分配（基于类型名 hash）
  - `loadTypeConfig()` / `saveTypeConfig()` — 持久化
- 节点渲染: 使用 `typeColors[entity.entity_type]` 读取颜色
- 浮动菜单新增 "类型管理" 入口，点击弹出类型管理面板（列表显示所有类型+数量+颜色选择器+合并按钮）

### Step 2: Local Graph 多跳深度

**前端:**

- `knowledgeStore.ts`: 新增 `focusDepth: number` 默认 1
- `KnowledgeSphere.tsx` 和 `KnowledgeSphere3D.tsx`:
  - 替换 1-hop 为 BFS 多跳计算（depth 1-3）
  - focus indicator 旁增加深度选择器 ① ② ③
- 2D 和 3D 同步支持

### Step 3: Orphans 孤岛节点

**后端:**

- 新增 Tauri command: `get_orphan_entities` → 查询 degree=0 的实体

**前端:**

- `knowledgeStore.ts`: 新增 `showOrphans: boolean` 默认 true
- `KnowledgeSphere.tsx` / `KnowledgeSphere3D.tsx`:
  - 孤岛节点（degree=0）渲染为灰色虚线边框、更低透明度
  - 放置在图谱外围（不参与物理或极小排斥力）
- `GraphSettingsPanel.tsx`: 新增 "显示孤岛实体" toggle

### Step 4: Properties 属性系统 Prompt 层面改进

**Python (`extract_service.py`):**

- `build_prompt()` 增强 JSON 模板，要求每个实体输出结构化 properties:
  ```json
  "properties": {
    "created_date": {"type": "date", "value": "2025-03-15"},
    "status": {"type": "text", "value": "active"},
    "version": {"type": "number", "value": 2.0},
    "tags": {"type": "tags", "value": ["ml", "production"]}
  }
  ```

**后端:**

- `write_extraction_result()`: 从 LLM JSON 读取 `properties` 字段存入 SQLite（当前硬编码 `'{}'`）

**前端:**

- `EntityPopover.tsx` / `EntityBrowser.tsx`: properties 按类型结构化渲染

---

## 阶段二：文件节点 + LLM 边 混合视图

### Step 5: 文件节点作为一等公民

**关键设计决策**: 文件节点与实体节点使用相同的圆形渲染，通过 `entity_type = "__file__"` 和浅灰色 `#9CA3AF` 区分语义角色。不使用方形/圆角矩形——统一圆形保持视觉一致性，类型颜色承担区分职责。

**后端 (Rust):**

- 新增 Tauri command: `get_wiki_files_for_graph` → 返回 wiki 目录下所有 .md 文件
- `get_graph_data()`: 合并文件节点（`entity_type = "__file__"`）到 GraphData
- `cache_relations`: 新增 `source_type = "wikilink"` 的关系类型
- 新增方法: `parse_wikilinks_from_file(path)` → 解析 .md 中的 `[[target]]` 模式

**前端类型:**

```typescript
interface Entity {
  is_file?: boolean;        // true when entity_type === "__file__"
  file_path?: string;       // relative path in wiki
  file_kind?: "md" | "canvas" | "image" | "pdf" | "other";
}
```

**前端渲染:**

- 文件节点: 2D 用圆形（`ctx.arc`）浅灰 `#9CA3AF`，3D 用 `sphereGeometry`
- 文件→实体边: 灰色虚线 "contains"
- 文件→文件边: 更细灰线 "wikilink"
- `GraphSettingsPanel`: 新增 "显示文件节点" toggle

### Step 6: Obsidian [[wikilink]] 解析

**后端:**

- 正则 `\[\[([^\]|#]+)(?:[|#][^\]]+)?\]\]` 解析 .md 文件
- 在 `nexus_extract_from_file` 完成后自动调用解析
- wikilink 关系: `source_type = "wikilink"`, `relation_type = "wikilink"`

**两种边来源对比:**

| 来源 | 类型 | 提取方式 | 确定性 |
|------|------|---------|--------|
| LLM 语义提取 | 实体↔实体 | LLM 分析文本内容 → JSON | 概率性（有置信度） |
| Wikilink 解析 | 文件↔文件 | 正则匹配 `[[...]]` | 确定性（100%准确） |
| 文件归属 | 文件→实体 | LLM 提取时记录 source_file | 确定性 |

---

## 阶段三：PixiJS + Barnes-Hut 性能升级

### Step 7: Barnes-Hut 空间划分 O(n²) → O(n log n)

**文件: `src/components/sphere/physics.ts`**

- 新增 `BarnesHutTree` 类（quadtree for 2D, octree for 3D）
- `tick()`: 替换 O(n²) repulsion 为 Barnes-Hut 树构建+质心近似
- theta 参数 0.8（精度/性能平衡）
- 完全向后兼容: SimNode/SimEdge/SimState 接口不变

### Step 8: PixiJS WebGL 替换 Canvas 2D

**新文件: `src/components/sphere/PixiRenderer.ts`**

- 基于 PixiJS v8 Application
- 节点: PIXI.Graphics 圆形+颜色环+Text 标签
- 边: 批量 Graphics 绘制（GPU 批量）
- 视口裁剪优化

**文件修改: `KnowledgeSphere.tsx`**

- 替换 Canvas 2D 为 PixiRenderer
- 交互逻辑（事件、坐标变换、hitTest）保持不变
- 物理 tick 逻辑不变

**依赖:** `npm install pixi.js@^8`

---

## 阶段四：Vditor 替换 Milkdown — Obsidian 级文档体验

### Step 9: 用 Vditor 替换 Milkdown（1-2 天）

**问题诊断**: 当前 Milkdown 编辑器有 3 个致命缺陷：① CSS 只有一行颜色设置，排版效果接近纯文本；② 工具栏按钮全部是假按钮（无 onClick 处理程序）；③ 只支持 WYSIWYG 单一模式，没有 Obsidian 的实时预览。Milkdown 要实现同等效果需大量手写插件和 CSS，性价比低。

**选型结论**: 采用 [Vditor](https://github.com/Vanessa219/vditor)（MIT 协议，10.8K+ stars），作者是思源笔记核心开发者。

**Vditor vs 当前状态:**

| 能力 | Milkdown（当前） | Vditor |
|------|:---:|--------|
| 编辑模式 | 仅 WYSIWYG | **IR** + WYSIWYG + **分屏预览** |
| 工具栏 | 假按钮 | 36+ 种操作，精简可配 |
| 排版/主题 | 零 CSS | **dark 主题**内置 |
| 代码高亮 | 无 | **36 套主题** |
| 大纲/TOC | 无 | 内置，**可收起**，快捷键唤出 |
| 中文优化 | 无 | 中英文**自动空格** + 中文标点替换 |
| HTML→MD 粘贴 | 无 | Word/Excel 粘贴**自动转** Markdown |
| 图片处理 | 无 | 拖拽/剪切板上传 → 回调接入 wiki 存储 |

**UI 设计 — 默认极简，按需展开:**

```
默认状态:
┌──────────┬──────────────────────────┬────┐
│ DocTree   │ H1 H2 H3 B I 🔗 🖼 📖 ···│    │  ← 单行 8 个按钮
│           │──────────────────────────│    │
│           │     Vditor IR 模式        │    │  ← 干净编辑区
│           │     即输即渲染            │    │
└──────────┴──────────────────────────┴────┘

点 ··· 展开完整工具栏，点 📖 滑出大纲:
┌──────────┬──────────────────────────┬──────────┐
│ DocTree   │ H1 H2 H3 B I S </> 🔗 🖼 │ 📖 大纲   │
│           │ ── 第二行 ──              │ ## 章节   │
│           │ 表格 引用 列表 撤销 重做     │ ### 子节  │
│           │ [IR] [所见] [分屏]         │ ## 章节   │
│           │──────────────────────────│          │
│           │     Vditor IR 模式        │ 点遮罩收起 │
└──────────┴──────────────────────────┴──────────┘
```

**实施细节:**

**依赖变更:**
```bash
npm uninstall @milkdown/kit @milkdown/react @milkdown/plugin-listener
npm install vditor
```
`react-markdown`/`remark-gfm`/`rehype-highlight` 保留（聊天消息渲染不变）。

**新文件: `src/components/knowledge/VditorEditor.tsx`** — 替代 MilkdownEditor：

```typescript
// 核心配置
const vditor = new Vditor(containerRef.current, {
  mode: "ir",              // 默认 IR 模式（= Obsidian 实时预览）
  theme: "dark",           // 匹配 app 深色主题
  height: "100%",
  content: content,        // 从磁盘读取的原始 MD 字符串
  outline: {
    enable: true,
    position: "right",     // 右侧滑出，默认隐藏，点 📖 才打开
  },
  input(value) {            // 编辑回调
    setContent(value);
    setDirty(true);
  },
  toolbar: [
    "headings", "bold", "italic", "|",
    "link", "image", "outline", "|",
    {
      name: "more",
      tip: "更多工具",
      hotkey: "⌘/",
      click() { /* 展开/收起第二行 */ },
    },
  ],
  preview: {
    hljs: { style: "github-dark-dimmed" },
    theme: { current: "dark" },
  },
  upload: {
    handler(files) {
      // 图片 → wiki 目录 → 返回 asset:// 路径
      return uploadWikiFiles(Array.from(files).map(f => f.name));
    },
  },
});
```
- Ctrl+S 保存逻辑不变
- `useEffect` 监听 `filePath` 变化重新加载内容不变

**修改: `KnowledgeEditor.tsx`** — 仅改一行：

```typescript
// 改前
import { MilkdownEditor } from "./MilkdownEditor";
// 改后
import { VditorEditor } from "./VditorEditor";
```
渲染: `<MilkdownEditor filePath={openFilePath} />` → `<VditorEditor filePath={openFilePath} />`

**删除:**
- `MilkdownEditor.tsx` + `MilkdownEditor.module.css`
- 不再需要 `markdown-typography.css`（Vditor 自带主题）
- 不再需要 `MarkdownViewer.tsx`（Vditor IR 模式 = 阅读视图）
- 不再需要编辑/预览切换按钮（Vditor 模式切换更完善）

**与提取管线零影响**: 编辑器只管写 `.md` 文件，Nexus 只管从 `.md` 文件提取实体。替换编辑器不影响任何后端逻辑。

---

## 后续扩展（用户自行写入）

### Step 10: 图片上传、预览与嵌入
### Step 11: PDF 上传与内嵌预览
### Step 12: 图片视觉提取（多模态 LLM）

---

## 验证

### 阶段一:
1. LLM 输出非标准类型（如 algorithm/framework）→ 图谱显示，颜色自动分配
2. Focus 深度 2/3 → 正确显示多跳邻居
3. 孤岛实体显示虚线圆 → 关闭 toggle 消失
4. entity detail 显示结构化 properties

### 阶段二:
5. wiki .md 文件显示为灰色圆形节点
6. 文件→实体有 "contains" 边
7. 互相 [[link]] 的文件间有 wikilink 灰线
8. "显示文件节点" toggle 正常工作

### 阶段三:
9. 700+ 节点 2D 流畅 60fps
10. 1000+ 节点 PixiJS WebGL 不掉帧
11. 3D Barnes-Hut octree 处理 500+ 节点

### 阶段四:
12. Vditor IR 模式即输即渲染，dark 主题排版质量接近 Obsidian
13. 大纲面板默认隐藏，点击 📖 滑出，点击遮罩收起
14. 完整工具栏默认折叠为 ··· 按钮，点击展开第二行
15. 模式切换 IR/所见即所得/分屏预览 正常工作

---

## 文件清单

| 文件 | 阶段 | 改动 |
|------|:----:|------|
| `src-tauri/src/models/knowledge.rs` | 1 | 枚举→String 别名 |
| `src-tauri/src/services/knowledge_service.rs` | 1-2 | 简化 parse、wikilink 解析、list_types/merge_types、properties 写入 |
| `src-tauri/src/commands/knowledge.rs` | 1-2 | 新增 commands |
| `src-tauri/src/services/extract_service.py` | 1 | Prompt 增强（自由类型 + properties） |
| `src/types/knowledge.ts` | 1-2 | 删除 ENTITY_TYPE_COLORS、新增 is_file 等 |
| `src/stores/knowledgeStore.ts` | 1-2 | typeColors/focusDepth/showOrphans/showFiles |
| `src/components/sphere/physics.ts` | 3 | BarnesHutTree 类 |
| `src/components/sphere/PixiRenderer.ts` | 3 | **新建** PixiJS 渲染器 |
| `src/components/sphere/KnowledgeSphere.tsx` | 1-3 | 多跳+孤岛+文件节点+PixiJS |
| `src/components/sphere/KnowledgeSphere3D.tsx` | 1-3 | 多跳+孤岛+文件节点 |
| `src/components/sphere/GraphSettingsPanel.tsx` | 1-2 | 新 toggle |
| `src/components/sphere/EntityPopover.tsx` | 1 | Properties 渲染 |
| `src/components/knowledge/EntityBrowser.tsx` | 1 | Properties 渲染 |
| `src/styles/markdown-typography.css` | ~~4~~ 废弃 | ~~排版 CSS~~ → Vditor 自带 dark 主题，不再需要 |
| `src/components/knowledge/MarkdownViewer.tsx` | ~~4~~ 废弃 | ~~只读阅读视图~~ → Vditor IR 模式取代 |
| `src/components/knowledge/MilkdownEditor.tsx` | 4 | **删除**，替换为 VditorEditor.tsx |
| `src/components/knowledge/MilkdownEditor.module.css` | 4 | **删除** |
| `src/components/knowledge/VditorEditor.tsx` | 4 | **新建** Vditor 编辑器组件 |
| `src/components/knowledge/KnowledgeEditor.tsx` | 4 | import MilkdownEditor → VditorEditor |
| `package.json` | 3-4 | +pixi.js v8, +vditor, -milkdown 相关 |
