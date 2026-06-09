# AI-Hel2 设计文档

## 概述

AI-Hel2 是一个 Tauri 桌面 AI 助手应用，核心交互界面为 3D 知识图谱球体 + 语音对话。从 AI-Hel 提取后端服务（Hermes Agent + 知识库），独立构建。

---

## 技术栈

| 层 | 选型 |
|---|---|
| 桌面框架 | Tauri 2.0（复用 AI-Hel 配置和插件） |
| 前端框架 | React 19 + TypeScript |
| 3D 引擎 | Three.js + @react-three/fiber + @react-three/drei |
| 球面布局 | d3-force（球面投影约束） |
| 状态管理 | Zustand |
| 样式 | CSS Modules |
| Markdown 编辑 | Milkdown（@milkdown/kit + commonmark + gfm） |
| 画板 | @excalidraw/excalidraw |
| 语音识别 | 浏览器 SpeechRecognition API（后备 Whisper.cpp） |
| 音频分析 | Web Audio API AnalyserNode |
| 通信 | Tauri invoke() + SSE |
| Rust 后端 | 复用 AI-Hel 的 services |
| 数据存储 | SQLite（rusqlite，复用 AI-Hel） |

---

## 页面结构

三页面应用，TabBar 切换：

```
TabBar: "对话球体" | "知识编辑" | "画板"
```

### 页面1：对话球体（主页）

A2 布局：左侧 3D 球体 + 右侧对话面板。

```
SphereChatView
├── KnowledgeSphere（R3F Canvas）
│   ├── SphereMesh（球体着色器 + 深绿渐变）
│   ├── EdgeLines（节点间连线）
│   └── AudioParticles（音频驱动粒子扩散层）
├── ChatPanel（右侧面板）
│   ├── MessageList（消息气泡）
│   ├── VoiceButton（唤醒/说话按钮）
│   └── TextInput（文字备选输入）
└── AudioAnalyzer（Web Audio 分析管线，非可视）
```

配色方案（3D 球体）：
- 球体基色：#0a1a0a → #0d2f0d（深绿渐变）
- 节点颜色：#6ee7b7（淡绿）
- 高亮节点：#f0c040（暖金，对比色）
- 连线颜色：rgba(100,200,150,0.3)（半透明绿）
- 粒子颜色：rgba(150,255,200,0.8) → #fff（淡绿渐变至白）

配色方案（应用 UI，对齐 AI-Hel 微信暗色主题）：
- 主背景：#1A1A1A（暗灰）
- 面板/侧栏：#2F2F2F
- 强调色：#07c160（微信绿）
- 主文字：#e6e6e6
- 次文字：#b3b3b3
- 弱文字：#808080
- 边框：#3A3A3A
- 悬停态：#3A3A3A
- 危险色：#fa5151
- 用户气泡：accent 绿底黑字
- AI 气泡：#2C2C2C 深灰底白字 + 边框

球面交互：
- 拖拽旋转 / 滚轮缩放
- 悬停节点高亮 + 名称
- 点击节点居中 + 右侧详情
- 对话提及自动高亮 + 旋转

音频驱动动画（B3 粒子扩散）：
- 安静：粒子悬浮节点附近做布朗运动
- 低频 → 球体缩放 + 大颗粒子扩散
- 中频 → 相关节点发光增强
- 高频 → 细碎粒子沿法线飞出
- 停声后粒子回缩（spring easing）

语音流程：
- 唤醒词："Hi Hel"
- 静音超时：默认 5 秒（可调 3-15 秒）
- 也支持空格键触发 + 发送按钮

### 页面2：知识编辑

顶部 Tab 切换：文档编辑 | 实体浏览。

**文档编辑 Tab：**
- 左侧文档树（AI-Hel 同款：ChevronRight/ChevronDown + FolderOpen + FileText/Palette/Image/File 图标）
- 右侧 Milkdown 编辑器 + frontmatter 面板
- 支持 @实体名 自动补全
- 文档树双击 .canvas → 跳转画板页
- 文档树支持多层级嵌套、展开折叠、搜索、新建

**实体浏览 Tab：**
- 实体搜索 + 分类列表（核心概念/普通实体）
- 实体详情面板（类型、置信度、描述、关联实体）
- "在球体中定位"按钮

### 页面3：画板

- 使用 @excalidraw/excalidraw，全屏
- 文件存取对接 Tauri 文件系统
- 文档树双击 .excalidraw 文件自动跳转此页面

---

## 数据流

```
用户语音 → SpeechRecognition → chatStore.send()
  → invoke("chat_send") → Hermes Agent → SSE 回复流
  → chatStore.update() + TTS 输出
  → AnalyserNode 提取频谱 → audioStore.spectrum[]
  → AudioParticles shader uniform（球体粒子动画）

对话提取新知识 → invoke("graph_data") 刷新
  → knowledgeStore 更新 → KnowledgeSphere 重绘节点

页面2 编辑文档 → invoke("wiki_save") → 触发 re-index
  → FileChangeEvent → knowledgeStore 刷新
  → 页面1 球体节点更新

页面3 画板 → 节点关联知识实体 → syncCanvasToGraph()
  → knowledgeStore 刷新 → 页面1 球体节点更新
```

---

## Rust 后端（从 AI-Hel 复用）

| 服务 | 来源 | 用途 |
|------|------|------|
| AgentManager | services/agent_manager.rs | Hermes Agent 对话 |
| KnowledgeService | services/knowledge_service.rs | 知识图谱索引与查询 |
| SessionService | services/session_service.rs | 对话历史持久化 |
| WikiService | services/wiki_service.rs | 文档树读写 |
| ConfigService | services/config_service.rs | 配置管理 |
| CanvasService | services/canvas_service.rs | 画板文件存取（适配 Excalidraw） |

新增命令：
- `chat_send`：发送消息给 Hermes Agent，返回 SSE 流
- `graph_data`：拉取知识图谱 JSON
- `voice_transcribe`：Whisper.cpp 转写（后备）

---

## 前端 Stores

| Store | 职责 |
|------|------|
| chatStore | 对话消息、流式状态、发送/接收 |
| knowledgeStore | 实体/关系/图谱数据（页面1+2 共用） |
| audioStore | 录音状态、频谱数据、播放状态 |
| uiStore | 当前页面、面板宽度、主题 |

---

## 性能目标

| 指标 | 目标 |
|------|------|
| 节点数 | 50-500 |
| 连线数 | 100-2000 |
| 粒子数 | 500-4000 |
| 帧率 | ≥55fps（WebView2） |
| 球体三角面 | ~256 段 |
| 安装包 | <15MB |

---

## 项目路径

```
D:\AI-Hel2\
├── src/                    # React 前端
│   ├── components/
│   │   ├── sphere/         # 3D 球体
│   │   ├── chat/           # 对话面板
│   │   ├── knowledge/      # 知识编辑
│   │   ├── canvas/         # 画板(Excalidraw 封装)
│   │   └── layout/         # AppShell + TabBar
│   ├── hooks/              # useAudioAnalyzer, useVoiceInput, useKnowledgeGraph
│   ├── stores/             # chatStore, knowledgeStore, audioStore, uiStore
│   ├── services/           # Tauri invoke 封装
│   └── types/              # TypeScript 类型定义
├── src-tauri/              # Rust 后端（从 AI-Hel 改造）
│   ├── services/           # 复用 AI-Hel 服务
│   ├── commands/           # Tauri 命令
│   └── models/             # 数据模型
├── docs/                   # 设计文档
└── package.json
```

---

## 语音后备方案

主力：浏览器 SpeechRecognition API
后备：Whisper.cpp（本地运行，免费，通过 voice.rs 命令调用）

切换触发条件：连续识别失败 3 次或用户手动切换设置。
