<p align="center">
  <img src="assets/social-preview.png" alt="AI-Hel2" width="320" />
</p>

# AI-Hel2 —— 你的伴随式智能体 / Your Companion Agent

> 不是另一个 AI 聊天工具。AI-Hel2 是一个**常驻桌面、持续学习、主动关联**的伴随式智能体——它在你工作时静静运行，将你的对话、文档、思考沉淀为一张不断生长的个人知识网络。
>
> Not another AI chat tool. AI-Hel2 is a **desktop-resident, continuously learning, proactively connecting** companion agent — it runs quietly while you work, transforming your conversations, documents, and thoughts into an ever-growing personal knowledge network.

## 为什么是 AI-Hel2？ / Why AI-Hel2?

市面上的 AI 助手大多是"用完即走"的对话框：你问，它答，然后一切归零。下一次对话时，它不记得你是谁，更不知道你上次在做什么。

**AI-Hel2 的不同之处**：它在本地持续运行，每一次对话、每一篇文档、每一个想法，都被自动提取、关联、沉淀为你的**个人知识图谱**。它不是你的工具，是你的**数字同事**——越用越懂你，越用越有用。

Most AI assistants are "use and lose" chat boxes: you ask, it answers, then resets to zero. The next conversation, it doesn't remember who you are, let alone what you were working on.

**What makes AI-Hel2 different**: it runs locally and continuously. Every conversation, every document, every idea is automatically extracted, linked, and crystallized into your **personal knowledge graph**. It's not your tool — it's your **digital colleague**. The more you use it, the more it understands you.

## 三大核心能力 / Three Core Capabilities

### 🧠 伴随式 Agent 引擎 / Companion Agent Engine

- **常驻桌面，随时唤醒 / Always-on, instant access**：`Alt+R` to唤出对话，右 Alt 按住说话，松开发送——像跟同事说话一样自然 / `Alt+R` to open chat, hold Right Alt to speak, release to send — as natural as talking to a colleague
- **多 Agent 协作平台 / Multi-Agent platform**：不是绑定单一模型，而是注册和管理多个 Agent，各自负责不同领域 / Not locked to a single model — register and manage multiple agents, each specializing in different domains
- **工具调用 + 网页搜索 / Tool calling + Web search**：Agent 能主动查资料、搜网页、操作知识库，不只是"聊天" / Agents actively research, search the web, and manipulate the knowledge base — not just "chat"
- **内嵌运行时，安装即用 / Embedded runtime, zero setup**：Hermes Agent 和 Python 运行时全部内置，无需额外安装 / Hermes Agent and Python runtime fully bundled — no extra installation needed

### 🔗 Nexus 知识引擎——让知识自己生长 / Nexus Knowledge Engine — Knowledge That Grows Itself

这是 AI-Hel2 最大的特色：**你的对话和文档会自动变成结构化的知识网络**。

This is AI-Hel2's defining feature: **your conversations and documents automatically become a structured knowledge network**.

- **自动知识提取 / Auto-extraction**：每段对话结束后，Nexus 引擎在后台通过 LLM 自动提取实体、概念和关系，写入知识图谱——你不需要手动整理 / After each conversation, Nexus uses LLMs to automatically extract entities, concepts, and relationships into the knowledge graph — no manual curation needed
- **跨文档推理 / Cross-document reasoning**：引擎自动发现跨文档的隐藏关联——"这篇笔记里的方案，和三个月前那篇会议记录里的思路其实是同一回事" / The engine discovers hidden connections across documents — "The approach in this note and the idea from that meeting three months ago are actually the same thing"
- **6 组智能维护 / 6 intelligent maintenance tasks**：健康检查、去重合并、文档归类、图谱分析、传递推理、冲突检测——知识库不是堆积，是生长 / Health checks, deduplication & merging, document classification, graph analytics, transitive reasoning, conflict detection — the knowledge base grows, not just accumulates
- **文档折叠视图 / Document folding view**：一键从细节中抽身，看到整个知识领域的宏观结构 / One click to step back from the details and see the macro structure of an entire knowledge domain

### 📊 知识可视化——看见你的思考 / Knowledge Visualization — See Your Thinking

- **力导向知识图谱 / Force-directed knowledge graph**：Barnes-Hut 四叉树物理引擎，节点自然分布，关联一目了然 / Barnes-Hut quadtree physics engine — nodes distribute naturally, connections at a glance
- **对话与图谱联动 / Chat-graph synchronization**：Agent 提到某个概念，图谱自动高亮旋转到对应节点——"说到哪儿，看到哪儿" / When the Agent mentions a concept, the graph auto-highlights and rotates to the corresponding node — "see what you're talking about"
- **全文搜索 + 实体面板 / Full-text search + Entity panel**：不只是搜关键词，而是搜到知识网络中的位置和上下文 / Not just keyword search — find the position and context within the knowledge network

<p align="center">
  <img src="assets/demo-screenshot.png" alt="AI-Hel2 Demo: Knowledge List + Knowledge Graph + AI Chat" width="100%" />
</p>

## 与典型 AI 工具的区别 / How It Compares

| | 普通 AI 聊天<br>Chat AI | 笔记软件<br>Note Apps | **AI-Hel2** |
|---|---|---|---|
| 记住上下文<br>Context | 单次对话内<br>Single session | 需手动整理<br>Manual | **自动沉淀到知识图谱<br>Auto-crystallized into graph** |
| 知识关联<br>Connections | 无<br>None | 手动双链<br>Manual backlinks | **LLM 自动提取 + 推理发现<br>LLM extraction + inference** |
| 运行方式<br>Presence | 用完即走<br>Ephemeral | 被动记录<br>Passive | **常驻桌面，持续伴随<br>Always-on companion** |
| 多 Agent<br>Multi-Agent | 无 | 无 | **注册管理多个 Agent<br>Multi-agent registry** |
| 数据归属<br>Data | 云端<br>Cloud | 本地/云端<br>Local/Cloud | **完全本地，隐私自主<br>Fully local, full privacy** |
| 语音交互<br>Voice | 部分支持<br>Partial | 无 | **PTT 语音 + TTS 播报<br>Push-to-talk + TTS** |

## 技术架构 / Architecture

```
┌─ Tauri v2 Shell (Rust) ──────────────────────────────┐
│  Window / System Tray / Global Shortcuts / File Watch │
├─ Frontend (React + TypeScript + Vite) ────────────────┤
│  D3-force Graph / Cherry Markdown / Excalidraw Canvas │
├─ Nexus Knowledge Engine (Rust + Python) ──────────────┤
│  SQLite / LLM Extraction / Barnes-Hut / Inference     │
├─ Hermes Agent v0.15.2 (Python, Embedded) ─────────────┤
│  AI Chat / Tool Calling / Web Search / KB Plugins     │
└───────────────────────────────────────────────────────┘
```

## 快速开始 / Quick Start

从 [Releases](https://github.com/fanbingqian/AI-Hel2/releases) 下载最新安装包，一键安装。

Download the latest installer from [Releases](https://github.com/fanbingqian/AI-Hel2/releases).

### 安装后三步走 / Three Steps After Install

1. 打开应用 → 注册本地账号 / Open app → Register a local account
2. 填入大模型 API Key（DeepSeek / OpenAI / Anthropic 等均可）/ Add your LLM API key (DeepSeek, OpenAI, Anthropic, etc.)
3. Agent 自动启动，知识库自动初始化 → 开始对话，知识开始生长 / Agent auto-starts, knowledge base auto-initializes → start chatting, knowledge starts growing

### 系统要求 / System Requirements

- Windows 10/11 x64
- 无需额外安装 Python 或 Git（已内置）/ No Python or Git installation required (bundled)

## 开发 / Development

### 环境准备 / Setup

```bash
git clone https://github.com/fanbingqian/AI-Hel2.git
cd AI-Hel2
npm install          # 安装前端依赖 / Install frontend dependencies
```

### 获取 Agent 运行时 / Obtain Agent Runtime

Agent 源码已在 `src-tauri/hermes-agent/` 中，但嵌入式 Python 运行时未包含在仓库中（体积过大）。从 [Releases](https://github.com/fanbingqian/AI-Hel2/releases) 下载最新安装包，安装后将以下目录拷贝到源码对应位置：

Agent source code is in `src-tauri/hermes-agent/`, but the embedded Python runtime is not included in the repository (too large). Download the latest installer from [Releases](https://github.com/fanbingqian/AI-Hel2/releases), install it, then copy these directories to the source tree:

```
<安装目录/install-dir>/data/hermes-agent/python/  →  src-tauri/hermes-agent/python/
<安装目录/install-dir>/data/hermes-agent/bash/    →  src-tauri/hermes-agent/bash/
```

> 或从 [Hermes Agent](https://github.com/NousResearch/hermes-agent) 官方仓库获取原始 Python 环境。
> Or obtain the Python environment from the [Hermes Agent](https://github.com/NousResearch/hermes-agent) official repository.

### 启动开发 / Run

```bash
npm run tauri dev    # 开发模式（热重载）/ Dev mode (HMR)
npm run tauri build  # 构建安装包 / Build installer
```

### 项目结构 / Project Structure

```
├── src/                     # React 前端源码 / Frontend source
├── src-tauri/
│   ├── src/                 # Rust 后端源码 / Backend source (Tauri commands)
│   ├── hermes-agent/        # Python Agent 源码 / Agent source (runtime excluded)
│   ├── icons/               # 应用图标 / App icons
│   ├── migrations/          # SQLite 数据库迁移 / DB migrations
│   └── Cargo.toml           # Rust 依赖 / Rust dependencies
├── docs/                    # 设计文档 / Design docs
├── scripts/                 # 辅助脚本 / Helper scripts
└── assets/                  # README 图片资源 / README images
```

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full text.

Copyright 2025-2026 AI-Hel2
