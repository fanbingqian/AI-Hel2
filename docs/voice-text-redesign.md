# 语音 + 文字输入重新设计方案

> 2026-05-28

## 一、当前系统全链路

### 1.1 发送链路（上行）

```
用户说话 / 打字
  │
  ├─ 语音: voiceStore → invoke("voice_listen_once") → Rust SttService → Python ASR daemon → 文字
  └─ 文字: 直接输入 textarea
  │
  └─→ chatStore.sendMessage(text)
        │ invoke("chat_completions", { messages: [{role:"user", content:text}], ... })
        ▼
      Rust chat.rs
        │ 1. build_context_snapshot() — 从知识图谱注入 system message
        │ 2. POST http://127.0.0.1:18642/v1/chat/completions
        ▼
      Python api_server.py
        │ system → ephemeral_system_prompt
        │ user/assistant → conversation_messages
        │ 创建 AIAgent → 内部 ReAct 循环 (思考→工具→思考→...→回复)
        ▼
      返回 SSE 流
```

**关键结论: 语音只是文本的另一种输入方式。Agent 不知道也无需知道文字来自键盘还是麦克风。**

### 1.2 返回链路（下行）

```
Agent ReAct 循环一次 sendMessage 内部:
  ├─ [思考] "用户问天气，需要搜索"    → message.thinking (ThinkingSection 折叠区)
  ├─ [工具] web_search("北京天气")    → message.toolCalls (ToolCallTimeline 折叠区)
  ├─ [思考] "确认数据，可以回复"      → message.thinking 追加
  └─ [回复] "今天北京晴朗，气温..."   → message.content (气泡正文)

Agent SSE 事件流:
  data: {"choices":[{"delta":{"role":"assistant"}}]}          → 角色声明
  data: {"choices":[{"delta":{"content":"今"}}]}              → 内容逐字
  event: hermes.tool.progress                                 → 工具进度
  data: {"tool":"web_search","toolCallId":"xxx","status":"running"}
  ...
  data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{...}}  → 结束
  data: [DONE]                                                → 流终止
```

**TTS 只读 message.content (最终回复气泡文字)，不读思考过程、不读工具调用。**

### 1.3 前端渲染结构（保持不变）

```
┌─────────────────────────────┐
│ 思考过程 ▾                  │  ← ThinkingSection (流式时展开，完成 1.5s 后自动折叠)
│ ┌─────────────────────────┐ │
│ │ 用户问天气，我需要搜索... │ │
│ └─────────────────────────┘ │
│                             │
│ ✓ web_search · 读取网页 · 2s│  ← ToolCallTimeline (运行时展开，完成 1.5s 后自动折叠)
│                             │
│ ┌─────────────────────────┐ │
│ │ 今天天气晴朗，温度...     │ │  ← Markdown content 气泡
│ └─────────────────────────┘ │
└─────────────────────────────┘
```

---

## 二、发现的 Bug 和问题

### 2.1 TTS "只读了几个字就没了" (严重 Bug)

**根因**: `appendDelta()` 创建 assistant 消息时从未设置 `isStreaming: true`。

```
chatStore.appendDelta("今"):
  → 创建消息 { content: "今", isStreaming: undefined }

ChatPanel TTS useEffect:
  → !undefined === true       ← 条件通过
  → speak("今")               ← 只读了第一个 delta!
  → spokenMsgIds.add(msgId)   ← 标记已读

chatStore.appendDelta("天"):
  → 更新消息 { content: "今天" }
  → spokenMsgIds 已有此 ID → TTS 从此跳过 → 后面的字再也读不到
```

**修复方案**:
1. `appendDelta` 新建/更新消息时设置 `isStreaming: true`
2. `chat:done` handler 中设置 `isStreaming: false`
3. ChatPanel TTS useEffect 条件改为 `isStreaming === false` (显式比较，不依赖 falsy)

### 2.2 TTS 3000 字符截断 (需改为分段续读)

**现状**: `voice.rs:107-116` 对超过 3000 字符的文本在最近句号处截断，剩余内容直接丢弃。

**修复方案**: 改为前端分段朗读：
1. 移除或大幅提高后端截断限制
2. `useTTS` 新增 `speakSegments(text)` 方法: 按句号/问号/感叹号分句
3. 顺序调用 TTS 合成 → 播放 → 等 ended 事件 → 下一段
4. 用户说话(barge-in)或发新消息时中止当前段及后续段

### 2.3 语音输入在 ChatPanel 中缺失

**现状**: `VoiceButton.tsx` 和 `useVoiceInput.ts` 存在但未被任何组件引用（死代码）。语音输入只在 KnowledgeSphere 3D 球体中通过 hold-to-talk 触发。

**修复方案**: 在 ChatPanel 输入栏增加语音模式，与文字输入共享同一入口。

### 2.4 TTS 默认开启

**现状**: ChatPanel 中 `ttsEnabled` 默认为 `true`，每条 assistant 消息自动触发语音播报，导致"输入文字也会有语音声音"。

**修复方案**: `ttsEnabled` 默认改为 `false`，用户手动开启。

### 2.5 状态管理碎片化

| 文件 | 管理的状态 | 问题 |
|------|-----------|------|
| `voiceStore.ts` | isListening, voiceText, voiceSource | — |
| `audioStore.ts` | status, isRecording, isSpeaking, spectrum | `isRecording` 和 `isListening` 表达同概念 |
| `useTTS.ts` | playingRef (本地 ref) | 不共享 |
| `ChatPanel.tsx` | ttsEnabled (本地 useState) | 不持久化 |

**修复方案**: 统一到 `voiceStore`，`audioStore` 合并入 `voiceStore`。

---

## 三、音色支持

### 3.1 当前音色

模型: `sherpa-onnx-vits-zh-ll`，5 种音色:

| ID | 名称 | 风格 |
|----|------|------|
| 0 | 苏映雪 (suyingxue) | 默认女声 |
| 1 | 顾年 (gunian) | 男声 |
| 2 | 傅诗雨 (fushiyu) | 女声 |
| 3 | 病娇 (bingjiao) | 女声 |
| 4 | 霸总 (bazong) | 男声 |

当前硬编码 speaker=0，用户无法选择。后端 `tts_speak` 虽然接受 `voice` 参数，但前端 `useTTS.ts` 从不传:

```typescript
const base64: string = await invoke("tts_speak", { text });  // 没有 voice 参数
```

### 3.2 音色选择方案

**改动点:**

| 文件 | 改动 |
|------|------|
| `SettingsPage.tsx` | 新增 "语音" 版块，列出 5 种音色，每项可点击试听 |
| `voice.rs` | 新增 `tts_preview` 命令，合成一句话 "你好，这是我的声音" |
| `settingsStore.ts` | 新增 `ttsSpeaker: number` (默认 0)，持久化 |
| `useTTS.ts` | `invoke("tts_speak", { text, voice: String(selectedSpeaker) })` |
| `ChatPanel.tsx` | TTS 开关旁显示当前音色名（如 "苏映雪 ▾"，点击弹音色选择下拉） |

**TTS 开关旁音色显示:**

```
[🔊 TTS:开] 苏映雪 ▾     ← 点击 ▾ 弹出音色列表
[🔊 TTS:关]               ← 关闭时不显示音色

音色下拉:
┌──────────────┐
│ ● 苏映雪 (女) │  ← 当前选中
│ ○ 顾年   (男) │
│ ○ 傅诗雨 (女) │
│ ○ 病娇   (女) │
│ ○ 霸总   (男) │
├──────────────┤
│ 🔊 试听       │  ← 点击播 "你好，这是我的声音"
└──────────────┘
```

**后端新增 `tts_preview` 命令:**

```rust
#[tauri::command]
pub async fn tts_preview(speaker: u8) -> Result<String, String> {
    tts_speak("你好，这是我的声音".to_string(), Some(speaker.to_string())).await
}
```

---

## 四、输入模式设计

### 4.1 核心原则

1. **文字和语音是对等输入方式**，共享一个输入区，通过模式切换
2. **TTS 播报是独立开关**，默认关闭，不与输入模式绑定
3. **状态集中管理**，单一 voiceStore 管理全链路
4. **视觉反馈完整**，录音/识别/播放每个阶段都有对应 UI

### 4.2 统一状态机

```
voiceStore 状态:

  idle ──→ listening ──→ transcribing ──→ preview ──→ sending ──→ idle
   ↑          │               │              │                       │
   │          └──→ idle ──────┘              │                       │
   │          (取消/超时)                     │                       │
   │                                         └──→ idle ─────────────┘
   │                                         (用户取消预览)           │
   └─────────────────────────────────────────────────────────────────┘

TTS 状态 (独立):

  idle ←→ speaking
           │
           └──→ interrupted ──→ listening (可选自动转入语音输入)
```

### 4.3 ChatPanel 输入栏改造

**文字模式 (默认)**:

```
┌─────────────────────────────────────────────────────────┐
│  [📎] [🖼] [🔊 TTS:关]               [🎤 语音输入]  │
│                                                         │
│  ┌─────────────────────────────────────────────────┐   │
│  │ 输入消息... (Enter 发送, Shift+Enter 换行)       │   │
│  └─────────────────────────────────────────────────┘   │
│                                            [发送]       │
└─────────────────────────────────────────────────────────┘
```

**语音模式 (点击 🎤 或 Ctrl+Space 切换)**:

```
┌─────────────────────────────────────────────────────────┐
│  [📎] [🖼] [🔊 TTS:开] 苏映雪 ▾        [⌨ 文字输入]   │
│                                                         │
│  ┌─────────────────────────────────────────────────┐   │
│  │              ⏺ 点击或按住说话                     │   │
│  │           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓                  │   │
│  │                00:03.2                           │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

**转录预览 (识别完成后)**:

```
┌─────────────────────────────────────────────────────────┐
│  ┌─────────────────────────────────────────────────┐   │
│  │ 今天天气怎么样                                  │   │  ← 可编辑
│  └─────────────────────────────────────────────────┘   │
│                                       [✕] [✓ 发送]     │
└─────────────────────────────────────────────────────────┘
```

### 4.4 交互规则

| 操作 | 行为 |
|------|------|
| 点击 🎤 按钮 | 切换到语音模式 |
| 点击录音区域（单击） | 开始录音，auto-stop 后自动识别 (豆包式) |
| 按住录音区域 | 持续录音，松手停止并识别 (微信式) |
| 识别完成 | 文本填入预览区，可编辑，再点发送 |
| 按 Escape | 取消当前录音/预览，回到 idle |
| 点击 ⌨ 按钮 | 切回文字输入模式 |
| Ctrl+Space | 切换文字/语音模式 |

---

## 五、TTS 协调规则

不改 MessageBubble / ThinkingSection / ToolCallTimeline 任何代码。协调逻辑在 voiceStore 方法和 ChatPanel useEffect 中实现，不新增独立编排文件。

| 场景 | TTS 行为 | 语音输入行为 |
|------|----------|-------------|
| 打字发送 → 收到回复 | TTS 开启时才播报，等 `isStreaming===false` 后播完整 content | 不启动 |
| 语音输入 → 预览 → 发送 → 回复 | 同上 | 预览中不 TTS |
| TTS 播报中用户说话 | 停止播放 (barge-in) | 可选自动进入录音模式 |
| 用户正在录音 | 阻塞不播报 | — |
| 回复超过 3000 字 | 分段朗读，每段等 ended 后播下一段 | — |
| 新消息到达时旧 TTS 还在播 | 停止旧播放，开始新播放 | — |

### 5.1 TTS 独立控制状态

TTS 与输入模式完全解耦，独立状态机：

```
TTS 状态:

  idle ←→ speaking
           │
           └──→ interrupted ──→ listening (可选自动转入语音输入)
```

| TTS 状态 | 行为 |
|----------|------|
| 关闭（默认） | assistant 回复只有文字，不发声 |
| 开启 | assistant 回复文字 + 自动语音播报 |
| 播放中被语音打断 | 停止播放，可选择是否自动进入语音输入 |
| 正在录音 | 阻塞 TTS，不播报 |

### 5.2 分段朗读流程

```
speakSegments(text):
  segments = splitBySentence(text, maxChars=200)  // ~3-5 句/段，首段延迟低
  for each segment:
    if (cancelled) break
    base64 = await invoke("tts_speak", { text: segment, voice: String(ttsSpeaker) })
    audio = play(base64)
    await audio.onended
```

---

## 六、状态流转伪代码

```typescript
type VoiceMode = "text" | "voice";
type VoiceStatus = "idle" | "listening" | "transcribing" | "preview" | "sending";
type TtsStatus = "idle" | "speaking" | "interrupted";

interface VoiceState {
  // 输入模式
  inputMode: VoiceMode;          // "text" | "voice"

  // 录音状态
  status: VoiceStatus;
  duration: number;              // 当前录制秒数

  // 识别结果
  transcribedText: string;       // 识别出的文本（可编辑）

  // TTS
  ttsEnabled: boolean;           // 默认 false
  ttsSpeaker: number;            // 0-4 音色选择
  ttsStatus: TtsStatus;
  ttsSegmentQueue: string[];     // 分段朗读队列
  ttsSegmentIndex: number;       // 当前段索引

  // 音频可视化
  spectrum: Float32Array;
  volume: number;

  // 错误
  error: string | null;

  // ── 动作 ──
  toggleInputMode: () => void;
  startRecording: () => Promise<void>;       // 单击开始
  stopRecording: () => Promise<void>;        // 单击停止/松手
  cancelRecording: () => void;
  confirmAndSend: (text?: string) => void;    // 确认发送
  toggleTts: () => void;
  setSpeaker: (id: number) => void;
  previewSpeaker: (id: number) => Promise<void>; // 试听
}
```

### 6.1 核心方法实现要点

**startRecording:**
1. 检查 `status === "idle"`，否则忽略
2. 调用 `invoke("voice_start_listening")` 或 `invoke("voice_listen_once")`
3. 设置 `status = "listening"`，启动计时器
4. 连接麦克风获取频谱数据 → `spectrum`

**stopRecording:**
1. 调用 `invoke("voice_stop_listening")`
2. 设置 `status = "transcribing"`
3. 等待后端返回文本 → `transcribedText`
4. 设置 `status = "preview"`

**confirmAndSend:**
1. 取 `text` 参数或 `transcribedText`
2. 调用 `chatStore.sendMessage(text)`
3. 设置 `status = "idle"`, 清空 `transcribedText`

> **ChatPanel vs KnowledgeSphere 差异:**
> - ChatPanel: `stopRecording()` → preview → 用户编辑/确认 → `confirmAndSend()`
> - KnowledgeSphere: `stopRecording()` → 直接 `confirmAndSend()`，跳过预览步骤

**toggleTts:**
1. `ttsEnabled = !ttsEnabled`
2. 如果关闭且正在播报 → `stopTTS()`

**speakSegments (内部):**
1. 按句号/问号/感叹号分句，每段 ≤200 字符 (~3-5 句)
2. 顺序合成+播放，等 `audio.onended` 再下一段
3. `cancelled` 标志位中止后续段

---

## 七、实施文件清单

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `stores/chatStore.ts` | 修改 | appendDelta 设置 isStreaming:true；chat:done 设置 isStreaming:false |
| `stores/voiceStore.ts` | 重写 | 统一状态机 (5 状态 + TTS)，合并 audioStore |
| `stores/audioStore.ts` | 删除 | 合并到 voiceStore |
| `stores/settingsStore.ts` | 修改 | 新增 ttsSpeaker, ttsEnabled 持久化字段 |
| `components/chat/ChatPanel.tsx` | 修改 | 增加输入模式切换 + TTS 条件修复 + ttsEnabled 默认 false |
| `components/chat/VoiceInput.tsx` | 新建 | 录音按钮 + 波形动画 + 计时器 + 转录预览 |
| `components/chat/VoiceButton.tsx` | 删除 | 被 VoiceInput 替代 |
| `hooks/useVoiceInput.ts` | 删除 | 逻辑已整合到 voiceStore，组件直接调用 store 方法 |
| `hooks/useTTS.ts` | 修改 | 默认关闭 + 传 voice 参数 + 分段朗读 |
| `components/settings/SettingsPage.tsx` | 修改 | 新增"语音"版块 (音色列表 + 试听) |
| `src-tauri/src/commands/voice.rs` | 修改 | 提高/移除截断限制，新增 tts_preview 命令 |
| `components/sphere/KnowledgeSphere.tsx` | 修改 | 适配新的 voiceStore API |
| `components/sphere/AudioParticles.tsx` | 修改 | 从 voiceStore 读取 spectrum/volume (原 audioStore 已合并) |

---

## 八、实施顺序

1. **修复 TTS Bug** — appendDelta isStreaming (用户当前问题)
2. **TTS 默认关闭 + 音色选择** — 立即可用
3. **分段朗读** — 替换 3000 字截断
4. **重建 voiceStore** — 统一状态管理，清理死代码
5. **ChatPanel 语音输入** — VoiceInput 组件 + 模式切换
6. **适配 KnowledgeSphere** — 更新 API 引用
