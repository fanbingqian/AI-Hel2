# 隐藏模式默认 + 同短边向左展开 实施方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 默认手机比例隐藏模式（9:16 = 390×693），展开为桌面比例三段式（16:9 = 1232×693），Chat 在两种模式下宽度相同（390px），窗口右边缘固定不动，仅向左扩展/向右收缩。

**Architecture:** 两种模式共享同一短边 693px（高度恒定），Chat 宽度统一为手机宽度（390px），消除模式切换时的 Chat 尺寸跳跃。展开时窗口左边界向左移动 842px，Chat 屏幕位置和尺寸完全不动。

**Tech Stack:** React 18 + TypeScript + Tauri v2 + Zustand + CSS Modules

---

## 尺寸设计（核心约束）

```
隐藏模式（默认 9:16）                三段式展开（16:9）
  短边 h=693                         短边 h=693（相同！）
  长边 w=390 (693×9/16)              长边 w=1232 (693×16/9)

┌──────┐ 390                        ┌─DocTree─┬──Main───┬──────┐ 1232
│      │                            │  260    │  582    │ 390  │
│ Chat │                            │         │         │ Chat │
│ 390  │ 693                        │         │         │ 390  │ 693
│      │                            │         │         │      │
└──────┘                            └─────────┴─────────┴──────┘
                                         ↑          ↑       ↑
                                    DocTree固定   flex:1  固定390
```

| 属性 | 隐藏模式 | 三段式 | 说明 |
|------|:--:|:--:|------|
| 窗口宽 | 390 | 1232 | 左边界移动 842px |
| 窗口高 | 693 | 693 | **相同，零垂直跳跃** |
| Chat 宽 | 390 (100%) | 390 (store) | **相同，零 Chat 跳跃** |
| Chat 位置 | 窗口右边缘 | 窗口右边缘 | **相同，Chat 不动** |
| 窗口最小宽 | 340 | 900 | |
| 窗口最大宽 | 500 | 无限制 | |

**为什么 Chat=390？** 390 是手机 9:16 的短边尺寸，也是日常对话最舒适的单栏宽度。三段式下 Chat 保持 390，Main Content（582px）放知识图谱/编辑器足够宽。

---

## 尺寸跳跃分析（修正后无跳跃）

```
场景                          Chat宽度  Chat位置  窗口高  窗口右边缘
─────────────────────────────────────────────────────────────────
启动（默认隐藏）               390      右边缘    693    基准
 ↓ 展开                      390      不变      693    不变 ← 零跳跃
 ↓ 折叠                      390      不变      693    不变 ← 零跳跃
 ↓ 用户拖拽Chat(→450)        450      不变      693    不变
 ↓ 折叠(保存450)             450→100%  不变      693    不变
 ↓ 展开(恢复450)             100%→450  不变      693    不变 ← Chat从100%切换到450px，窗口随之适配

唯一变化：窗口左边界左右移动，Chat 完全不动
```

---

## 文件清单

| 文件 | 操作 | 职责 |
|------|:--:|------|
| `src-tauri/tauri.conf.json` | 修改 | 窗口 390×693, minWidth 340, minHeight 500 |
| `src/stores/uiStore.ts` | 修改 | `panelCollapsed`→true, `chatPanelWidth`→390 |
| `src/components/aihel/AiHelPage.tsx` | 修改 | 双 ref 防跳跃 + 向左展开 + 首次 mount 不 resize |
| `src/components/auth/AuthForms.module.css` | 修改 | `.card` 适配 390px 小窗 |
| `src/components/chat/ChatPanel.tsx` | 检查 | 确认自适应逻辑（无需修改） |
| `src/components/layout/AppShell.tsx` | 检查 | TabBar compact 已正确（无需修改） |

---

## Task 1: 窗口默认尺寸

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: 修改窗口配置**

```json
// src-tauri/tauri.conf.json → app.windows[0]
{
  "title": "AI-Hel2",
  "width": 390,
  "height": 693,
  "minWidth": 340,
  "minHeight": 500,
  "center": true,
  "resizable": true,
  "decorations": true
}
```

改动：`width: 1200→390`, `height: 750→693`, `minHeight: 600→500`。`minWidth: 340` 不变。

- [ ] **Step 2: 验证编译**

```bash
npx tsc --noEmit --pretty
```

---

## Task 2: uiStore 默认值

**Files:**
- Modify: `src/stores/uiStore.ts`

- [ ] **Step 1: `panelCollapsed` 默认 true + `chatPanelWidth` 默认 390**

```typescript
// src/stores/uiStore.ts

// 约第 49 行 — Chat 宽度默认改为手机宽度
chatPanelWidth: 390,

// 约第 62 行 — 默认隐藏模式
panelCollapsed: true,
```

- [ ] **Step 2: `chatPanelWidth` setter clamp 调整为 340-500**

```typescript
// 当前: Math.min(500, Math.max(220, w))
// 改为:
setChatPanelWidth: (w) =>
  set({ chatPanelWidth: Math.min(500, Math.max(340, w)) }),
```

下限从 220 改为 340（Win11 装饰窗口最小宽度）。

- [ ] **Step 3: 验证编译**

```bash
npx tsc --noEmit --pretty
```

---

## Task 3: AiHelPage —— 双 ref 防跳跃 + 向左展开 + 首次不 resize

**Files:**
- Modify: `src/components/aihel/AiHelPage.tsx`

这是核心。尺寸跳跃的根因是 `prevSizeRef` 只有一个——折叠时写入展开尺寸，展开时又读出来，但第一次展开时 `prevSizeRef` 存的是隐藏尺寸（390），导致窗口无法恢复到正确的展开宽度（1232）。

**修正：拆成两个 ref**——`expandedSizeRef` 只存展开态尺寸，`hiddenSizeRef` 只存隐藏态尺寸，互不污染。

- [ ] **Step 1: 修改 import 添加 `LogicalPosition`**

```typescript
// src/components/aihel/AiHelPage.tsx 约第 4 行
import { LogicalSize, LogicalPosition } from "@tauri-apps/api/dpi";
```

- [ ] **Step 2: 替换窗口缩放常量**

```typescript
// ── Dynamic window sizing on collapse/expand ──
// Dimensions: hidden=9:16 (390×693), expanded=16:9 (1232×693), same short side
const CHAT_W = 390;             // Chat width = phone width, same in both modes
const CHAT_MIN_W = 340;         // Win11 decorated window minimum
const CHAT_MAX_W = 500;         // Chat max width
const HIDDEN_H = 693;           // short side, same in both modes
const EXPANDED_W = 1232;        // 693 × 16/9
const EXPANDED_MIN_W = 900;     // minimum when expanded
const MIN_H = 500;              // minimum window height

// Two separate refs — prevent size contamination between modes
const expandedSizeRef = useRef<{ w: number; h: number; x: number; y: number }>({
  w: EXPANDED_W, h: HIDDEN_H, x: 0, y: 0,
});
const hiddenSizeRef = useRef<{ w: number; h: number; x: number; y: number }>({
  w: CHAT_W, h: HIDDEN_H, x: 0, y: 0,
});
const mountedRef = useRef(false);
```

- [ ] **Step 3: 替换整个 useEffect**

```typescript
useEffect(() => {
  const win = getCurrentWindow();
  let cancelled = false;

  (async () => {
    if (!mountedRef.current) {
      // First mount — Tauri already created window at config size (390×693).
      // Just capture initial position and set mode-appropriate constraints.
      mountedRef.current = true;
      const size = await win.innerSize();
      const pos = await win.outerPosition();
      hiddenSizeRef.current = { w: size.width, h: size.height, x: pos.x, y: pos.y };

      if (panelCollapsed) {
        await win.setMinSize(new LogicalSize(CHAT_MIN_W, MIN_H));
        await win.setMaxSize(new LogicalSize(CHAT_MAX_W, 4000));
      } else {
        await win.setMinSize(new LogicalSize(EXPANDED_MIN_W, MIN_H));
      }
      return;
    }

    if (panelCollapsed) {
      // ── COLLAPSE: window shrinks rightward, right edge fixed ──
      const pos = await win.outerPosition();
      const size = await win.innerSize();
      if (cancelled) return;

      // Save expanded size BEFORE collapsing (for accurate restore later)
      expandedSizeRef.current = { w: size.width, h: size.height, x: pos.x, y: pos.y };

      // Read target hidden size (user may have resized hidden window previously)
      const targetW = Math.min(CHAT_MAX_W, Math.max(CHAT_MIN_W, hiddenSizeRef.current.w));
      const targetH = Math.max(MIN_H, hiddenSizeRef.current.h);

      await win.setMinSize(new LogicalSize(CHAT_MIN_W, MIN_H));
      await win.setMaxSize(new LogicalSize(CHAT_MAX_W, 4000));
      // Shrink first, then reposition so right edge stays fixed
      await win.setSize(new LogicalSize(targetW, targetH));
      const deltaX = size.width - targetW;
      await win.setPosition(new LogicalPosition(pos.x + deltaX, pos.y));
    } else {
      // ── EXPAND: window grows leftward, right edge fixed ──
      const pos = await win.outerPosition();
      const size = await win.innerSize();
      if (cancelled) return;

      // Save hidden size (user may have resized hidden window)
      hiddenSizeRef.current = { w: size.width, h: size.height, x: pos.x, y: pos.y };

      // Restore expanded size
      const targetW = Math.max(EXPANDED_MIN_W, expandedSizeRef.current.w);
      const targetH = Math.max(MIN_H, expandedSizeRef.current.h);

      await win.setMaxSize(null);
      await win.setMinSize(new LogicalSize(EXPANDED_MIN_W, MIN_H));
      // Move left first, then expand — right edge stays fixed
      const deltaX = targetW - size.width;
      await win.setPosition(new LogicalPosition(Math.max(0, pos.x - deltaX), pos.y));
      await win.setSize(new LogicalSize(targetW, targetH));
    }
  })();

  return () => { cancelled = true; };
}, [panelCollapsed]);
```

注意：这个 effect **不再依赖 `chatPanelWidth`**。Chat 宽度的变化不影响窗口大小——窗口由 ref 中保存的尺寸控制。用户拖拽 Chat 只影响三段式下的 Chat 列宽，与窗口折叠/展开逻辑解耦。

- [ ] **Step 4: 展开时 docTreeCol 宽度适配新布局**

当前 `docTreeWidth` 默认 260。新方案下 Main Content 宽度 = 1232 - 390(Chat) - 260(DocTree) = 582px。合理。

如果 Chat 被用户拖到 500（最大），Main = 1232 - 500 - 260 = 472px。仍够用。

无需修改 `docTreeWidth` 默认值。

- [ ] **Step 5: 验证编译**

```bash
npx tsc --noEmit --pretty
```

---

## Task 4: ChatPanel 确认自适应

**Files:**
- Check: `src/components/chat/ChatPanel.tsx`（无需修改）

- [ ] **Step 1: 确认逻辑正确**

当前代码约第 156 行：
```typescript
<div className={styles.panel}
  style={panelCollapsed
    ? { width: "100%", minWidth: 0 }
    : { width: chatPanelWidth, minWidth: chatPanelWidth }}
>
```

- 隐藏模式 (`panelCollapsed=true`): `width: 100%` → 填满 390px 窗口 ✓
- 三段式 (`panelCollapsed=false`): `width: 390` (store 默认) → 与隐藏模式宽度一致 ✓
- 用户拖拽 Chat 到 450 再折叠：`width: 100%` → 窗口缩到 450，Chat 填满 → 视觉一致 ✓

**无需修改。**

---

## Task 5: 登录/注册表单适配 390px 小窗

**Files:**
- Modify: `src/components/auth/AuthForms.module.css`

- [ ] **Step 1: `.card` 改为响应式**

当前 `width: 380px` 在 390px 窗口下只有 5px 边距，太紧。

```css
.card {
  background: var(--bg-primary, #1A1A1A);
  border: 1px solid var(--border, #3A3A3A);
  border-radius: 12px;
  padding: 28px 20px;
  width: calc(100% - 24px);
  max-width: 380px;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.6);
  display: flex;
  flex-direction: column;
  gap: 20px;
}
```

改动：`width: 380px` → `width: calc(100% - 24px); max-width: 380px`，`padding: 40px 36px` → `padding: 28px 20px`。

- [ ] **Step 2: `.wideCard` 同样响应式（API 配置页）**

```css
.wideCard {
  width: calc(100% - 24px);
  max-width: 600px;
  max-height: 85vh;
  overflow-y: auto;
  padding: 24px 20px;
}
```

- [ ] **Step 3: 验证编译**

```bash
npx tsc --noEmit --pretty
```

---

## Task 6: TabBar + AppShell 确认

**Files:**
- Check: `src/components/layout/AppShell.tsx`（无需修改）
- Check: `src/components/layout/TabBar.tsx`（无需修改）

- [ ] **Step 1: 确认 TabBar compact 默认激活**

```typescript
// AppShell.tsx
<TabBar compact={activePage === "aihel" && panelCollapsed} />
```

`activePage` 默认 `"aihel"`，`panelCollapsed` 默认 `true` → compact 默认激活 ✓

- [ ] **Step 2: 确认展开时 TabBar 恢复**

`panelCollapsed` → `false` → compact 关闭，TabBar 显示文字标签 ✓

---

## Task 7: 验证完整流程

- [ ] **Step 1: 启动**

```bash
npm run tauri dev
```

预期：
- 390×693 手机比例窗口，居中
- TabBar 图标模式（◉ ◉ Agent▼ 头像 ⚙）
- Chat 填满窗口，"说 'Hi Hel' 唤醒我"
- `[›]` 按钮在左侧

- [ ] **Step 2: 登录流程**

Splash → 登录/注册 → API 配置，全部在 390px 窗口内：
- 表单不溢出，padding 足够
- 输入框可正常使用

- [ ] **Step 3: 展开**

点击 `[›]`：
- 窗口向左平滑增长到 1232×693
- Chat 390px 不变，位置不变
- DocTree 260px 左侧出现
- Main Content 中间出现（知识图谱）
- TabBar 恢复文字标签
- `[‹]` 按钮在 Chat 左侧

- [ ] **Step 4: 折叠**

点击 `[‹]`：
- 窗口向右收缩到 390×693
- 所有元素恢复隐藏模式
- 无闪烁、无跳跃

- [ ] **Step 5: 自由推拉**

- 隐藏模式：可拉至 340×500(min) ∼ 500×∞(max)
- 三段式：可拉至 900×500(min)，无上限
- 右键 Chat 边缘拖拽可调整 Chat 宽度(340-500)
- 展开/折叠时 Chat 宽度保持不变
- 无白边/黑边闪现

---

## 审查检查

1. **Spec coverage:** 默认隐藏(Task1+2)、向左展开(Task3)、同短边零跳跃(Task3 双 ref)、Chat 同宽(Task2+4)、登录适配(Task5)、验证(Task7)
2. **Placeholder scan:** 无
3. **Type consistency:** `LogicalPosition` 与 `LogicalSize` 同源 `@tauri-apps/api/dpi`
4. **尺寸跳跃:** 双 ref 方案消除——`expandedSizeRef` 和 `hiddenSizeRef` 独立存储，互不污染
