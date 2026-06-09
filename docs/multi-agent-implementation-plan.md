# HTTP Agent 多路切换 + 自动检测 + Nexus 工具注入

## Context

AI-Hel2 当前只有一个 Hermes Agent（Python 子进程 HTTP :18642），且配置体系（config.yaml）和 Agent 生命周期（AgentManager）都是围绕"只有一个 Hermes"设计的。

目标是把 AI-Hel2 变成**通用壳**：不管是 Hermes、OpenClaw、DeepSeek 还是任何 OpenAI 兼容的 Agent，都是 agents.json 里平等的一条记录。用户只装了 OpenClaw？直接配上去就能用壳 + Nexus 知识库。Hermes 只是出厂预装的一个默认选项，不是什么特殊存在。

设计文档: `docs/multi-agent-switching.md`

## 核心原则

1. **agents.json 是唯一的 Agent 配置源** — 包含所有 Agent（含 Hermes），每个 Agent 自带模型列表
2. **Hermes 和其他 Agent 平等** — 它只是 `agent_type: "hermes_builtin"`，进程管理是这个 type 的实现细节
3. **没有强制兜底** — 用户可以删掉 Hermes，只用 DeepSeek + OpenClaw，应用正常工作
4. **出厂预装** — agents.json 不存在时，seed 一条 Hermes 记录，纯首次使用引导

---

## agents.json 统一格式

```json
{
  "agents": [
    {
      "id": "hermes-builtin",
      "display_name": "Hermes Agent (内置)",
      "agent_type": "hermes_builtin",
      "enabled": true,
      "config": {
        "base_url": "http://127.0.0.1:18642/v1",
        "models": ["claude-sonnet-4-6", "deepseek-v4-pro"]
      }
    },
    {
      "id": "openclaw",
      "display_name": "OpenClaw",
      "agent_type": "openclaw",
      "enabled": true,
      "config": {
        "base_url": "http://127.0.0.1:18789/v1",
        "api_key": "<从 openclaw.json 自动读取>",
        "models": ["claude-sonnet-4-6"]
      },
      "detected": true,
      "detected_path": "~/.openclaw/openclaw.json"
    },
    {
      "id": "deepseek",
      "display_name": "DeepSeek V4",
      "agent_type": "openai_compatible",
      "enabled": true,
      "config": {
        "base_url": "https://api.deepseek.com/v1",
        "api_key": "sk-xxx",
        "models": ["deepseek-v4-pro", "deepseek-chat"]
      },
      "added_manually": true
    }
  ],
  "default_agent_id": "hermes-builtin"
}
```

所有 Agent 同一格式。`agent_type` 决定内部用什么方式通信，但从用户/配置角度看没区别。

---

## 实现步骤

### Step 1: Rust — Agent trait + 类型定义

**新建 `src-tauri/src/services/agents/mod.rs`**
- `AgentConfig` struct: 对应 agents.json 中一条记录
- `AgentInfo` struct: 脱敏后给前端（无 api_key）
- `AgentType` enum: `HermesBuiltin`, `OpenClaw`, `OpenAICompatible`
- `AgentRegistry` struct: 持有所有 Agent 实例 + AgentStore + AgentDetector

**新建 `src-tauri/src/services/agents/agent_interface.rs`**
- `ChatEvent` enum: Delta {content, reasoning}, ToolProgress, Done, Error
- `ChatOptions` struct: model, session_id
- `AgentInterface` trait: id(), display_name(), chat_stream(), health_check(), capabilities()

### Step 2: Rust — Agent 实现

**新建 `src-tauri/src/services/agents/hermes_builtin.rs`**
- 将现有 `HermesAgentService` 重构为实现 `AgentInterface`
- 唯一的特殊处理: 需要 AgentManager 管理 Python 子进程生命周期
- `health_check()` → POST `{base_url}/health`
- `chat_stream()` → POST `{base_url}/chat/completions` + SSE → `ChatEvent` 流
- 从 agents.json 的 config.models 读可用模型列表（不再依赖 config.yaml）

**新建 `src-tauri/src/services/agents/openai_compatible.rs`**
- 通用实现，覆盖: DeepSeek / OpenClaw / GLM / Kimi / OpenAI 等
- 构造时直接从 AgentConfig 拿 base_url + api_key + models
- `chat_stream()` → POST `{base_url}/chat/completions` + SSE → `ChatEvent` 流
- **Nexus tools 注入**: 每次请求自动附带 5 个 nexus_* 工具 JSON Schema
- **tool_calls 拦截**: 解析响应中的 tool_calls → 调本地 Nexus API (`:18643`) → 结果注入 messages → 继续生成

**OpenClaw 特殊处理**: agent_type 记为 `openclaw`，检测时自动从 `~/.openclaw/openclaw.json` 读 token + port，内部仍用 `OpenAICompatible` 实现通信，只是配置来源不同。

### Step 3: Rust — 自动检测 (AgentDetector)

**新建 `src-tauri/src/services/agent_detector.rs`**
- 三层兜底检测:
  - Layer 1: 已知安装路径（1ms）— 各平台的默认安装位置
  - Layer 2: `where` / `which` 命令（100ms）
  - Layer 3: 登录 Shell 探测（4s 超时）
- 检测指纹:
  - Claude Code: `claude` 可执行文件路径
  - Codex: `codex` 可执行文件路径
  - OpenClaw: `~/.openclaw/openclaw.json` + health ping
- 检测结果合并到 agents.json: 新发现的追加，已有检测到的更新 version/available 状态
- 启动时后台异步执行，不阻塞 UI

### Step 4: Rust — AgentRegistry + AgentStore

**新建 `src-tauri/src/services/agent_store.rs`**
- agents.json 读写，路径 `~/.ai-hel2/agents.json`
- 首次启动: agents.json 不存在 → seed 一条 Hermes 出厂记录

AgentRegistry 方法:
- `load_persisted()`: 从 agents.json 加载，所有 Agent 立即可用
- `background_scan()`: 后台异步三层扫描 → 追加新 Agent → emit `agents:updated`
- `add_manual(config)`: 用户手动添加 → 写入 agents.json
- `remove(id)`: 删除
- `set_enabled(id, bool)`: 启用/禁用
- `set_default(id)`: 设置默认 Agent
- `list()`: 返回 `Vec<AgentInfo>`（脱敏）
- `get(id)`: 返回 `Box<dyn AgentInterface>` 实例

Agent 实例按需创建：从 AgentConfig 构造对应的 AgentInterface 实现。Hermes 的 Python 子进程由 AgentManager 管理（如果用户启用了 Hermes）。

### Step 5: Rust — Nexus /mcp 端点

**新建 `src-tauri/src/services/nexus_mcp.rs`**
- 在现有 Nexus HTTP server（`:18643`）上增加 `/mcp` 路由
- MCP JSON-RPC: `tools/list`, `tools/call`
- Claude Code 检测到后自动注册: `claude mcp add nexus http://localhost:18643/mcp`

**新建 `src-tauri/src/services/agents/nexus_tools.rs`**
- 5 个工具定义，OpenAI function calling 格式
- 被 MCP 端点和 openai_compatible.rs 共用

### Step 6: Rust — chat.rs 路由改造

修改 `chat_completions` 命令:
- 增加 `agent_id` 参数 → 从 AgentRegistry 获取 Agent → `agent.chat_stream()`
- 事件名不变 (`chat:delta`, `chat:done`, `chat:tool-progress`, `chat:error`)
- 不再依赖全局 `AgentState`（现行代码中的 `State<'_, AgentState>`），改为从 AgentRegistry 查找

### Step 7: Rust — lib.rs 启动流程

```
App 启动
  ├─ AgentStore::load_or_seed() → agents.json 就绪
  ├─ AgentRegistry::load_persisted() → 所有 Agent 实例化
  ├─ emit "agents:updated" → 前端立刻显示 Agent 列表
  ├─ 如果 Hermes 在列表中且 enabled → AgentManager::start() 拉起子进程
  └─ 后台: AgentRegistry::background_scan() → 发现新 Agent → emit "agents:updated"
```

### Step 8: Rust — Session 表 + agent_id

修改 `session_service.rs`:
- `ALTER TABLE sessions ADD COLUMN agent_id TEXT`
- 新建 session 时记录当前使用的 agent_id

### Step 9: 前端 — 状态管理

**新建 `src/hooks/useAgentRegistry.ts`**: 监听 `agents:updated`，管理 agents 列表 + activeAgentId

修改 `chatStore.ts`:
- `sendMessage` 传 agentId → 后端路由到对应 Agent
- model 从 activeAgent 的 models 列表获取，不再硬编码
- Session 列表显示每条对话用的哪个 Agent

### Step 10: 前端 — UI 组件

**新建 `AgentSwitcher`** — 聊天页顶部下拉框，切换 Agent + 模型

**新建 `AgentSettings`** — 设置页管理面板:
- 列表: 名称 | 类型 | 状态(●运行中/✓已检测/○离线) | 模型数 | 启用开关
- "重新检测" 按钮
- "手动添加" → 弹窗（名称 / baseURL / API Key / 模型列表）
- 删除（手动添加的）/ 禁用（检测到的）

### Step 11: 前端 — 改造 ApiSetupWizard（首次引导 + 检测 + 配模型）

修改 `src/components/auth/ApiSetupWizard.tsx`，从单步改为两步:

**Step 1: 填写 API Key**（现有逻辑，微调）
- 用户选 provider + 填 Key
- 同时在后台异步启动 Agent 检测（调 `re_detect_agents` 或直接走 lib.rs 后台扫描）
- 点 "下一步" 进入 Step 2 时检测结果已就绪

**Step 2: Agent 检测结果 + 模型配置**
```
┌──────────────────────────────────────────────────────────┐
│ Step 2: 检测本地 Agent                                    │
│                                                          │
│ 以下是在你电脑上发现的 AI Agent，可直接切换使用：           │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │ ● Hermes Agent (内置)                              │  │
│  │   本地服务 :18642                          ✓ 已就绪  │  │
│  │   模型: [claude-sonnet-4-6 ▼] [+ 添加模型]         │  │
│  │ ● Claude Code                                      │  │
│  │   ~/.local/bin/claude                   v1.0.37    │  │
│  │   检测到已安装，需配置 MCP 后可调用 Nexus            │  │
│  │ ○ OpenClaw                                         │  │
│  │   未检测到安装                          可稍后手动添加 │  │
│  │ ● DeepSeek V4 (从 Step 1 自动创建)                  │  │
│  │   api.deepseek.com                        ✓ 已配置  │  │
│  │   模型: [deepseek-v4-pro ▼] [+ 添加模型]           │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  [← 上一步]                                  [开始使用]   │
└──────────────────────────────────────────────────────────┘
```

关键逻辑:
- 检测在 Step 1 时后台跑，到 Step 2 时结果已就绪
- Step 1 填了 Key 的 provider → 自动在 Step 2 显示为可用 Agent（OpenAI 兼容类型）
- 每个 Agent 可展开配模型列表（下拉选择 + 自定义添加）
- 内置 Hermes 永远显示 "已就绪"
- 点 "开始使用" → 写入 agents.json + config.yaml → 进入主 AppShell
- 不影响现有 `AuthStage` 枚举，在 ApiSetupWizard 内部用 `wizardStep` 状态机管理

修改 `ChatPanel.tsx`: 顶部加 AgentSwitcher
修改 `SettingsPage.tsx`: 导航加 "Agent 管理"

---

## 文件清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `src-tauri/src/services/agents/mod.rs` | **新建** | AgentConfig + AgentRegistry |
| `src-tauri/src/services/agents/agent_interface.rs` | **新建** | Trait + ChatEvent + ChatOptions |
| `src-tauri/src/services/agents/hermes_builtin.rs` | **新建** | Hermes 实现（重构自 hermes_agent.rs） |
| `src-tauri/src/services/agents/openai_compatible.rs` | **新建** | 通用 HTTP + Nexus tools 注入 |
| `src-tauri/src/services/agents/nexus_tools.rs` | **新建** | 5 个工具 JSON Schema |
| `src-tauri/src/services/agent_detector.rs` | **新建** | 三层检测 |
| `src-tauri/src/services/agent_store.rs` | **新建** | agents.json 读写 + seed |
| `src-tauri/src/services/nexus_mcp.rs` | **新建** | /mcp JSON-RPC 端点 |
| `src-tauri/src/services/mod.rs` | **修改** | 模块声明 |
| `src-tauri/src/commands/agent.rs` | **修改** | +5 命令 |
| `src-tauri/src/commands/chat.rs` | **修改** | agent_id 路由 |
| `src-tauri/src/lib.rs` | **修改** | 新启动流程 |
| `src-tauri/src/services/session_service.rs` | **修改** | +agent_id |
| `src/hooks/useAgentRegistry.ts` | **新建** | 前端状态 |
| `src/components/chat/AgentSwitcher.tsx` | **新建** | 切换下拉 |
| `src/components/settings/AgentSettings.tsx` | **新建** | 管理面板 |
| `src/types/chat.ts` | **修改** | ChatSession +agentId |
| `src/stores/chatStore.ts` | **修改** | +agentId, model 动态化 |
| `src/components/auth/ApiSetupWizard.tsx` | **修改** | 双步引导: 填Key → 检测Agent+配模型 |
| `src/components/chat/ChatPanel.tsx` | **修改** | +AgentSwitcher |
| `src/components/settings/SettingsPage.tsx` | **修改** | +导航项 |

**总计: 11 新建, 10 修改**

## 不做

- 跨 Agent 共享上下文
- CLI Agent spawn 子进程模式
- 设置页现有的 "大模型配置" section 移入 Agent 管理面板内（每个 Agent 各自配模型）

## 验证

1. `cargo build` + `npm run build` 无错误
2. 首次启动 → Step 1 填 Key → Step 2 展示检测结果 → 配模型 → 进入主界面
3. Step 2 自动显示内置 Hermes + 从 Step 1 Key 创建的 Agent（如 DeepSeek）
4. 后续启动直接进主界面，AgentSwitcher 显示所有已配置 Agent
5. 删掉 Hermes，只留 DeepSeek → 聊天正常，Nexus 可查询
6. 检测到 OpenClaw → 自动出现在列表中，配置从 openclaw.json 读取
7. Nexus tools 注入 → Agent 可调用 nexus_search 查知识库
8. `/mcp` 端点 → `tools/list` 返回 5 个工具
9. AgentSwitcher 切换 → 模型列表跟随变化 → 对话历史保留
