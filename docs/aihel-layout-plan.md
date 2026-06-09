# AI-Hel2 UI 布局重构方案

## Context

当前 AI-Hel2 是 5 个独立页面（对话球体/知识编辑/画板/AI Word/设置），通过 TabBar 切换。每个页面独占屏幕空间，功能分离但割裂。用户要求合并前三者为统一的三段式布局，并增加桌面平刘海悬浮条。

## 目标架构

### 页面变化

```
改前: [对话球体] [知识编辑] [画板] [AI Word] [⚙]
改后: [AI Hel2] [AI Word] [⚙]
```

### AI Hel2 页面 — 三段式布局（全展开）

```
┌──────────────────────────────────────────────────────────┐
│ [AI Hel2] [AI Word]              [Agent选▼] [👤用户] [⚙] │  ← TabBar
├──────────┬────────────────────────┬─────────────────────┤
│ DocTree  │ Main Content (主内容)   │ Chat (对话栏)        │
│ (知识库) │                        │ (固定宽度,透明玻璃)   │
│          │ ┌─ 2D/3D 知识图谱 ───┐ │                     │
│ 文档列表  │ │ ForceGraphWrapper  │ │  ChatPanel          │
│ 实体列表  │ └────────────────────┘ │                     │
│          │ ┌─ 知识编辑 ─────────┐ │                     │
│ 可推拉    │ │ CherryEditor/edit │ │  透明玻璃背景         │
│ 可隐藏    │ └────────────────────┘ │  固定不隐藏           │
│          │ ┌─ 画板 ─────────────┐ │                     │
│          │ │ Excalidraw         │ │                     │
│          │ └────────────────────┘ │                     │
├──────────┴────────────────────────┴─────────────────────┤
│  ← 隐藏分割线, 可拖拽调整宽度 →                           │
└─────────────────────────────────────────────────────────┘
```

```
DocTree 列排布（从上到下）：
┌──────────────────┐
│ 🔍 搜索文档...    │  ← 搜索框（保留不变）
├──────────────────┤
│ [📄] [📁+] [📝+] [🎨+] │  ← 图标按钮一排
├──────────────────┤
│ 📁 文件夹          │
│   📝 文档.md       │  ← 文档列表（保留不变）
│   🎨 画板.canvas   │
│   ...             │
└──────────────────┘
```

- 搜索框：顶部，不变
- 图标按钮：搜索框下方，一排四个（上传、新建文件夹、新建文档、新建画板）
- 文档列表：按钮下方

**Main Content 切换方式**——通过 DocTree 点击文件自动切换：
- 点击 `.md` 文件 → 打开编辑器（CherryEditor）
- 点击 `.canvas` 文件 → 打开画板（Excalidraw）
- 未打开文件时默认显示知识图谱（graphViewMode 控制 2D/3D）

**DocTree 工具栏**（图标按钮一排）：
| 图标 | 功能 |
|------|------|
| 📄 | 上传文件 |
| 📁+ | 新建文件夹 |
| 📝+ | 新建 Markdown 文档 → 自动打开编辑器 |
| 🎨+ | 新建画板 → 自动打开 Excalidraw |
| 🔍 | 搜索过滤（保留不变） |

- **TabBar 完整保留**: `[AI Hel2] [AI Word]` 两个 tab + 右侧 `[Agent 下拉选择▼] [👤用户头像] [⚙设置]`
- AI Word 仍是独立 tab 页面，不变
- 设置仍是齿轮按钮，切换 `activePage = "settings"`，不变
- Agent 选择器和用户登录状态仍在 TabBar 右侧，不变
- **分割线**: DocTree↔Main 和 Main↔Chat 之间使用**不可见的推拉手柄**（鼠标悬停时光标变为 ↔，可拖拽调整宽度）
- DocTree 和 Main 各有一个最小宽度，拖到小于该宽度时自动折叠隐藏
- Chat 宽度固定可调但不隐藏

### Main Content 内容切换

Main Content 显示什么由 DocTree 文件点击自动驱动，无独立 tab。

**模式 1：知识图谱（默认，无文件打开时）**

```
┌─────────────────────────────────────────────┐
│ [FloatingMenu: 2D/3D切换 设置 Lint 实体列表] │  ← 右上悬浮
│                                              │
│         2D D3 力导图 或 3D Three.js           │
│         (ForceGraph2DWrapper / 3DWrapper)     │
│           节点/边/标签/缩放/拖拽               │
│                                              │
│ [适配窗口] [图例面板] [详情面板]               │
└─────────────────────────────────────────────┘
```
- 浮动菜单 + 设置面板 + Lint 面板 + 实体列表面板（同现在 SphereChatView 图谱区域）
- 图例面板（左上角）
- 节点详情面板（右上角，点击节点弹出）

**模式 2：文档编辑器（点击 `.md` 文件）**

```
┌──────────────────────────────────────────┐
│                                           │
│         Cherry Markdown 编辑器              │
│         (支持 wikilink [[链接]])            │
│                                           │
├──────────────────────────────────────────┤
│ 文件名.md       [编辑/阅读/分屏] [☀] [保存]  │
│                ● 已修改 (dirty时显示)       │
└──────────────────────────────────────────┘
```
- 同现在 KnowledgeEditor 的编辑器区——CherryEditor 原封不动搬过来
- 模式切换（编辑/阅读/分屏）、暗色/亮色主题、Ctrl+S/Cmd+S 保存、wikilink 点击跳转
- 底部状态栏：文件名 + 模式按钮 + 主题按钮 + 保存按钮 + 修改状态

**模式 3：画板（点击 `.canvas` 文件）**

```
┌──────────────────────────────────┐
│                                    │
│      Excalidraw 无限画布           │
│      (填满整个 Main Content)       │
│      (自动保存)                    │
│                                    │
└──────────────────────────────────┘
```

| 操作 | Main Content 显示 |
|------|------------------|
| 未打开任何文件（默认） | 模式 1：知识图谱 |
| 点击 `.md` 文件 | 模式 2：CherryEditor |
| 点击 `.canvas` 文件 | 模式 3：Excalidraw |
| 点击新建画板按钮 | 创建文件 → 模式 3 |
| 点击新建文档按钮 | 创建文件 → 模式 2 |
| 点击浮动菜单「2D」「3D」 | 切回模式 1 |

### 折叠控制

DocTree 和 Main Content 作为**一个整体**来折叠——一个按钮同时控制两者的显示/隐藏。

```
展开:  ┌─ DocTree ─┬─ Main ───────────┬── Chat ──┐
       │           │                  │          │
       │           │            [‹ →] │  透明玻璃  │   ← 折叠按钮
       └───────────┴──────────────────┴──────────┘
          DocTree↔Main 推拉手柄(隐藏)
                                    (Main↔Chat) 推拉手柄(隐藏)

隐藏:  ┌────────────────────────────┬── Chat ──┐
       │                            │          │
       │  暗色背景                   │  透明玻璃  │   ← [→ ‹] 展开按钮
       └────────────────────────────┴──────────┘
```

- **一个折叠按钮**（Main 右上角 `‹ →`），点击后 DocTree+Main 整体向右滑入 Chat 背后隐藏
- **展开按钮**（`→ ‹`），点击后滑出恢复
- **DocTree↔Main 之间**：推拉手柄调整文档栏和主内容的比例
- **Main↔Chat 之间**：推拉手柄调整主内容和对话的比例
- Chat 始终在最右侧，不隐藏

### AI Hel2 页面 — 隐藏模式 TabBar（紧凑图标模式）

匹配现有暗色风格（`#2C2C2C` 背景，`#b3b3b3` 文字，`#07c160` 绿色强调）：

```
┌──────────────────────────────────────────┐
│ [◉] [◉]            [Agent ▼] [⏺] [⚙]   │  ← 40px 高，同现有 TabBar
└──────────────────────────────────────────┘
```

- **AI Hel2 tab**: SVG 图标（聊天气泡形状），绿色高亮表示当前活跃
- **AI Word tab**: SVG 图标（地球/网络形状）
- **Agent 选择下拉**: 保持现有样式（`#2C2C2C` bg, `#3A3A3A` border, `#b3b3b3` text, 11px, 110px max-width）
- **用户头像**: 保持现有绿色圆形头像（`#07c160` bg, 26×26px）+ 用户名隐藏（仅显示头像）
- **设置齿轮**: 保持现有 SVG 齿轮图标

展开模式恢复文字标签 + 用户名。

### 桌面平刘海 (flat bangs)

280×52px，屏幕顶部居中，常驻最前，无边框，暗色玻璃背景。

**空闲状态**：
```
┌──────────────────────────────────────────┐
│ [🤖] AI-Hel2 · 在线                [⏺]  │
└──────────────────────────────────────────┘
```

**AI 工作中（悬停展开任务步骤，52→230px）**：
```
┌──────────────────────────────────────────┐
│ [🤖] 🔍 搜索网页… 12s               [⏺]  │
├──────────────────────────────────────────┤
│ ✓ 🔍 搜索关键词                          │
│ ◌ 📄 提取网页内容                        │
└──────────────────────────────────────────┘
```

**交互**：
- 双击 → 弹出隐藏模式 Chat 透明玻璃窗口
- 再双击 → 收回（回到之前的窗口状态：隐藏模式或三段式）
- 右键 → 退出 AI-Hel2

### Main Content 状态保持

折叠/展开和模式切换**纯视觉变化**，不刷新内容：

| 操作 | Main Content 状态 |
|------|------------------|
| 三段式打开 `notes.md` 编辑中 → 折叠隐藏 | 编辑器内容保持，不关闭 |
| 隐藏模式 → 展开三段式 | Main Content 回到上次编辑的 `notes.md` |
| 三段式看 2D 图谱 → 折叠 → 点画板 → 回三段式 | 图谱状态保留（缩放位置/选中节点） |
| 平刘海双击 → 隐藏模式 | Main Content 状态保持 |
| 平刘海再双击 → 三段式 | Main Content 状态恢复，无刷新 |

实现：AiHelPage 不卸载 MainContent，只用 CSS `display`/`width` 控制显隐，React 组件不销毁。

## 实现方案

### 第一步：新建 AiHelPage 主组件

**文件**: `src/components/aihel/AiHelPage.tsx` (新建)

职责：管理三段式布局。包含三个子区域：
- 左：`DocTree`（复用 `src/components/knowledge/DocTree.tsx`）
- 中：`MainContent`（新组件，切换 2D图谱/3D图谱/编辑器/画板）
- 右：`ChatPanel`（复用 `src/components/chat/ChatPanel.tsx`，加玻璃背景）

内部切换逻辑：
```
DocTree.onFileOpen(path, fileKind)
    → AiHelPage 判断扩展名:
        .md      → setMainContentMode("editor"), setOpenFilePath(path)
        .canvas  → setMainContentMode("canvas"), setOpenFilePath(path)
        其他     → 保持当前模式
    → MainContent 根据 mainContentMode 渲染对应组件
```

状态（uiStore 新增）：
- `panelCollapsed: boolean` — DocTree+Main 是否折叠（替代 leftPanelVisible/mainPanelVisible，二者绑定）
- `mainContentMode: "graph2d" | "graph3d" | "editor" | "canvas"`
- `openFilePath: string | null`

### 第二步：新建 MainContent 组件

**文件**: `src/components/aihel/MainContent.tsx` (新建)

根据 `mainContentMode` 切换渲染：
- `"graph2d"` → `ForceGraph2DWrapper`（复用）
- `"graph3d"` → `ForceGraph3DWrapper`（复用）
- `"editor"` → `KnowledgeEditor` 的编辑区（CherryEditor + FilePreview）
包含浮动菜单（FloatingMenu）、设置面板（GraphSettingsPanel）、Lint面板、实体列表面板——从 SphereChatView 搬过来。

### 第三步：修改 DocTree 工具栏

**文件**: `src/components/knowledge/DocTree.tsx`

在现有搜索框下方增加一排图标按钮（复用现有 `handleUpload`/`handleNewFile`/`handleCreateFolder` 逻辑，新增 `handleNewCanvas`）：

```
搜索框 (保留)
[上传] [新建文件夹] [新建文档] [新建画板]  ← 新增一行
文档列表 (保留)
```

点击文档/画板文件时，调用 `onFileOpen` 回调，AiHelPage 根据扩展名设置 `mainContentMode`（`.md`→editor，`.canvas`→canvas）。

### 第四步：修改 TabBar

**文件**: `src/components/layout/TabBar.tsx`

```typescript
const tabs = [
  { id: "aihel", label: "AI Hel2" },
  { id: "aiword", label: "AI Word" },
];
// settings gear stays
```

移除 `"sphere"`, `"knowledge"`, `"canvas"` 三个 tab，画板并入 Main Content。

### 第五步：修改 AppShell

**文件**: `src/components/layout/AppShell.tsx`

- 移除对 SphereChatView、KnowledgeEditor、CanvasPage 的直接引用
- 新增 `"aihel"` 路由 → `<AiHelPage />`
- `"settings"` 和 `"aiword"` 保持不变

### 第六步：修改 PageId 类型

**文件**: `src/types/index.ts`

```typescript
export type PageId = "aihel" | "aiword" | "settings";
```

### 第七步：ChatPanel 玻璃透明样式

**文件**: `src/components/chat/ChatPanel.module.css`

添加玻璃透明效果：
```css
.panel {
  background: rgba(26, 26, 26, 0.75);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
  border-left: 1px solid rgba(255, 255, 255, 0.06);
}
```

### 第八步：AiHelPage 布局 CSS

**文件**: `src/components/aihel/AiHelPage.module.css` (新建)

三段式布局：
- `display: flex; height: 100%;`
- 左栏：`width: 260px; flex-shrink: 0;`（可推拉）
- 中栏：`flex: 1; min-width: 0;`
- 右栏：`width: chatPanelWidth; flex-shrink: 0;`
- 推拉 resizer：同现有 PanelResizer 组件

### 第九步：桌面平刘海窗口 (Tauri)

**文件**: `src-tauri/src/lib.rs`

在 `setup` 中创建平刘海窗口：
```rust
use tauri::WebviewWindowBuilder;
use tauri::WebviewUrl;

let pill = WebviewWindowBuilder::new(
    app,
    "pill",
    WebviewUrl::App("pill.html".into())
)
.title("AI-Hel2")
.inner_size(280.0, 52.0)
.position(screen_width/2 - 140, 0)
.decorations(false)
.transparent(true)
.always_on_top(true)
.skip_taskbar(true)
.build()?;
```

**文件**: `src/pill.html` (新建) + `src/pill.tsx` (新建)

平刘海 UI：左侧猫图标，中间"AI-Hel2 · 在线"，右侧状态灯。双击→通过 Tauri IPC 显示主窗口并切换到隐藏模式。

### 第十步：类型和 Store 调整

**文件**: `src/stores/uiStore.ts`

新增：
- `panelCollapsed: boolean` — DocTree+Main 折叠状态
- `mainContentMode: "graph2d" | "graph3d" | "editor" | "canvas"`
- `openFilePath: string | null`

**文件**: `src/stores/knowledgeStore.ts`

保持现有状态不变（graphViewMode, showLintPanel, showEntityList, settingsOpen）。

## 关键约束

1. **不修改现有组件的功能逻辑**——只调整布局引用
2. **DocTree、ChatPanel、ForceGraphWrapper、CanvasPage 等核心组件保持不变**
3. **设置页面不变、AI Word 页面不变**
4. **玻璃透明仅影响 ChatPanel 的 `.panel` 背景，不影响内部元素**
5. **平刘海是新增功能，通过 Tauri webview 实现**

## 文件清单

| 文件 | 操作 |
|------|------|
| `src/types/index.ts` | 修改 PageId |
| `src/components/layout/TabBar.tsx` | 修改 tabs |
| `src/components/layout/AppShell.tsx` | 修改路由 |
| `src/components/aihel/AiHelPage.tsx` | **新建** |
| `src/components/aihel/AiHelPage.module.css` | **新建** |
| `src/components/aihel/MainContent.tsx` | **新建** |
| `src/components/chat/ChatPanel.module.css` | 修改（玻璃效果） |
| `src/stores/uiStore.ts` | 新增字段 |
| `src/pill.html` | **新建** |
| `src/pill.tsx` | **新建** |
| `src-tauri/src/lib.rs` | 新增 pill 窗口 |
| `src-tauri/tauri.conf.json` | 可能需加 CSP |

## 验证

1. `npm run tauri dev` 启动，确认 TabBar 显示 `[AI Hel2] [AI Word] [⚙]`
2. AI Hel2 页面三段式布局正常显示
3. 推拉隐藏左右栏正常
4. 中栏可切换：图谱2D/3D、编辑器、画板
5. Chat 有透明玻璃效果
6. 设置和 AI Word 页面不变
7. 平刘海窗口在桌面顶部显示
8. 双击平刘海弹出隐藏模式对话窗
