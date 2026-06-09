# AI-Hel2 多 Agent 自动检测 + 切换方案

## 背景

AI-Hel2 当前只有一个 Hermes Agent（本地子进程 HTTP :18642），如果要支持用户根据场景自由切换 Agent（Claude Code / OpenClaw / DeepSeek 等），需要解决三个问题：

1. **自动检测**：GUI 应用 PATH 不全，如何发现用户已安装的 CLI 工具？
2. **统一接口**：CLI 子进程 vs HTTP API 两种通信方式，Shell 如何无差别调用？
3. **共享上下文**：切换 Agent 后，新 Agent 如何看到之前的对话历史？

参考 HermesPet Win 的三层检测机制和跨 Agent 共享记忆方案。

---

## 1. Agent 检测 + 持久化（发现即注册，随时可用）

### 1.1 核心原则

```
检测 ≠ 启动时一次性行为
检测 = 发现 → 持久化配置 → 立刻出现在 Agent 列表中 → 随时可用
```

**四种触发时机**：

| 时机 | 行为 |
|------|------|
| **App 首次启动** | 三层扫描 → 发现 Agent → 持久化到 `agents.json` |
| **后续启动** | 先从 `agents.json` 加载（瞬时）→ 后台异步复检更新状态 |
| **设置页"重新检测"** | 清缓存 + 重新三层扫描 → 发现新 Agent → 追加到 `agents.json` |
| **用户手动添加** | 填 baseURL/API Key/model → 直接写入 `agents.json`，不走检测 |

### 1.2 持久化格式

```json
// ~/.ai-hel2/agents.json
{
  "agents": [
    {
      "id": "hermes-builtin",
      "display_name": "Hermes Agent (内置)",
      "agent_type": "hermes_builtin",
      "enabled": true,
      "config": { "base_url": "http://localhost:18642/v1", "model": "" }
    },
    {
      "id": "claude_code",
      "display_name": "Claude Code",
      "agent_type": "claude_code",
      "enabled": true,
      "config": {
        "executable_path": "/home/user/.local/bin/claude",
        "version": "1.0.37"
      }
    },
    {
      "id": "openclaw",
      "display_name": "OpenClaw",
      "agent_type": "openclaw",
      "enabled": true,
      "config": {
        "base_url": "http://localhost:18789",
        "token_source": "config_file",
        "port": 18789
      }
    },
    {
      "id": "deepseek",
      "display_name": "DeepSeek V4",
      "agent_type": "openai_compatible",
      "enabled": true,
      "config": {
        "base_url": "https://api.deepseek.com/v1",
        "api_key": "sk-xxx",
        "model": "deepseek-v4-pro"
      },
      "added_manually": true
    }
  ]
}
```

**关键**：一旦写入 `agents.json`，Agent 就永久出现在用户的 Agent 列表中。下次启动直接读 JSON，不需要重新检测。后台异步复检只更新 `version` 和 `available` 状态，不影响已注册的 Agent。

### 1.3 启动流程

```
App 启动
  │
  ├─ 第 0 步（瞬时）: 读取 ~/.ai-hel2/agents.json
  │    → 所有已注册的 Agent 立即可用
  │    → 内置 Hermes Agent 强写入（兜底，永远存在）
  │
  └─ 第 1 步（后台异步，不阻塞 UI）:
       AgentDetector::scan_all()
         ├─ 对每个 agent_type 做三层扫描
         ├─ 发现新 Agent → 追加到 agents.json
         ├─ 已注册 Agent 状态更新（version/available）
         └─ 完成后 emit "agents:updated"
```

用户打开设置页时，Agent 列表已经全部就绪——不是"正在检测中..."的空白页。

### 1.4 三层兜底检测

```
AgentDetector::find_executable(name)
  │
  ├─ Layer 1: 已知安装路径（1ms，纯文件检查）
  │    claude: ~/.local/bin/claude, ~/.local/share/claude/versions/X.Y.Z/claude
  │    codex:  ~/.local/bin/codex, /opt/homebrew/bin/codex
  │    hermes: ~/.hermes/config.yaml
  │    openclaw: ~/.openclaw/openclaw.json
  │
  ├─ Layer 2: where/which (100ms)
  │    Windows: where claude
  │    Unix:    which claude
  │
  └─ Layer 3: 登录 Shell 探测（4s 超时）
       zsh -lic 'command -v claude'
       powershell -Command "Get-Command claude"
```

### 1.5 四种 Agent 的检测指纹

| Agent | 检测方式 | 配置文件 | 通信方式 |
|-------|---------|---------|---------|
| **Claude Code** | 足迹 → where → shell | 无（CLI 自带认证） | spawn `claude -p` 子进程 |
| **OpenClaw** | `~/.openclaw/openclaw.json` + `/health` ping | openclaw.json（含 token/port） | HTTP SSE :18789 |
| **外部 Hermes** | `~/.hermes/config.yaml` + `/health` ping | config.yaml | HTTP SSE :8642 |
| **内置 Hermes** | 不检测（始终可用） | AI-Hel2 自身管理 | HTTP SSE :18642 |
| **OpenAI 兼容** | 不检测（用户手动添加） | 用户填 baseURL/API Key/model | HTTP SSE |

### 1.6 新文件

- `src-tauri/src/services/agent_detector.rs` — 三层检测逻辑
- `src-tauri/src/services/agent_store.rs` — agents.json 读写 + 持久化管理

---

## 2. AgentInterface 统一契约

### 2.1 Trait 定义

```rust
// src-tauri/src/services/agents/agent_interface.rs

#[async_trait]
pub trait AgentInterface: Send + Sync {
    /// 标识
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;

    /// 生命周期
    async fn start(&self) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn health_check(&self) -> Result<AgentStatus, String>;

    /// 核心：流式聊天（Shell 只调这个）
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<Box<dyn Stream<Item = ChatEvent> + Send + Unpin>, String>;

    /// 能力声明
    fn capabilities(&self) -> AgentCapabilities;
}
```

### 2.2 统一事件流

```rust
pub enum ChatEvent {
    Delta { content: String, reasoning: Option<String> },
    ToolProgress { tool: String, label: String, status: String },
    Done { usage: Usage, session_id: Option<String> },
    Error { message: String, retryable: bool },
}
```

四种 Agent 的输出全部转换到这个统一的 `ChatEvent` 流，Shell 不需要知道后面是 CLI 子进程还是 HTTP SSE。

### 2.3 新文件

- `src-tauri/src/services/agents/agent_interface.rs` — trait + ChatEvent + ChatOptions
- `src-tauri/src/services/agents/mod.rs` — AgentRegistry

---

## 3. 四种 Agent 实现

### 3.1 内置 Hermes Agent（已有代码重构）

```
src-tauri/src/services/agents/hermes_agent.rs
```

- 复用现有的 `HermesAgentService`，重构为实现 `AgentInterface`
- 通信：HTTP POST `localhost:18642/v1/chat/completions` → SSE 流
- 启动：由 AgentManager 管理子进程生命周期

### 3.2 Claude Code Agent（新增）

```
src-tauri/src/services/agents/claude_code_agent.rs
```

- 通信：`Command::new("claude").arg("-p").stdin(pipe).stdout(pipe)` spawn 子进程
- 解析：stdout jsonl 逐行解析
- 上下文：`build_prompt(messages)` 把完整对话历史拼成文本（参考 HermesPet `buildPrompt$2`）
- 活动转发：解析 tool_use → 转为 `ChatEvent::ToolProgress`
- 超时：5 分钟无输出 → 自动 kill

### 3.3 OpenClaw Agent（新增）

```
src-tauri/src/services/agents/openclaw_agent.rs
```

- 通信：HTTP POST `localhost:18789/chat/completions`（OpenAI 兼容 SSE）
- 配置：自动读取 `~/.openclaw/openclaw.json` 拿 token + port
- API Key：从配置文件自动提取（用户零填表）
- 启动：`Command::new("openclaw").args(["daemon", "start"])` 自动拉起

### 3.4 OpenAI 兼容 Agent（通用 HTTP Agent）

```
src-tauri/src/services/agents/openai_compatible_agent.rs
```

- 通信：标准 OpenAI 兼容 HTTP SSE（/v1/chat/completions）
- 配置：用户手动填 baseURL + API Key + model
- 覆盖：DeepSeek、GLM、Kimi、MiniMax、OpenAI 等所有 OpenAI 兼容服务商
- 复用 ProviderPreset 预设表（移植自 HermesPet）

### 3.5 新文件总览

```
src-tauri/src/services/agents/
├── mod.rs                       # AgentRegistry
├── agent_interface.rs           # Trait + 类型定义
├── hermes_agent.rs              # 重构自 hermes_agent.rs
├── claude_code_agent.rs         # 新增
├── openclaw_agent.rs            # 新增
├── openai_compatible_agent.rs   # 新增（通用 HTTP）
└── provider_presets.rs          # 移植自 HermesPet ProviderPreset
```

---

## 4. 跨 Agent 上下文（不需要实现）

跨 Agent 共享记忆层**不做**。每个对话独立绑定一个 Agent，对话历史保留在 Session 中，切换 Agent 时只需确保新 Agent 能调用 Nexus 工具即可（见 §7.4），不需要在两个 Agent 之间传递上下文。

### 4.1 对话级 Agent 绑定

```sql
-- Session 表新增字段
ALTER TABLE sessions ADD COLUMN agent_id TEXT DEFAULT 'hermes-builtin';
```

```
对话 A: agent_id = "hermes-builtin"   → 用 Hermes 回答
对话 B: agent_id = "claude_code"      → 用 Claude Code 回答
对话 C: agent_id = "deepseek"         → 用 DeepSeek 回答

用户在对话 B 中切换到 DeepSeek：
  → 对话 B 的 agent_id 更新为 "deepseek"
  → 历史 messages 仍在 Session 中
  → DeepSeek 收到对话 B 的历史 messages（标准 OpenAI messages 数组）
  → DeepSeek 可通过 Nexus tools 查询知识图谱
```

每个对话的 messages 是独立存储的，切换 Agent 后只传当前对话的历史 — 不涉及跨 Agent 记忆共享。

---

## 5. AgentRegistry — 持久化 + 动态注册

### 5.1 架构

```rust
// src-tauri/src/services/agents/mod.rs

pub struct AgentRegistry {
    agents: RwLock<HashMap<String, Box<dyn AgentInterface>>>,
    store: AgentStore,                    // agents.json 读写
    detector: AgentDetector,              // 三层检测
}

impl AgentRegistry {
    /// 从 agents.json 加载所有已注册 Agent（瞬时，不检测）
    pub fn load_persisted(&mut self) -> Vec<AgentInfo>

    /// 后台异步扫描，发现新 Agent 追加到 agents.json
    pub async fn background_scan(&self)

    /// 用户手动触发重新检测
    pub async fn re_detect(&mut self) -> Vec<AgentInfo>

    /// 用户手动添加 OpenAI 兼容 Agent
    pub fn add_manual(&mut self, config: ManualAgentConfig) -> Result<AgentInfo, String>

    /// 移除 Agent（仅限手动添加的，检测到的只能禁用）
    pub fn remove(&mut self, id: &str) -> Result<(), String>

    /// 启用/禁用某个 Agent
    pub fn set_enabled(&mut self, id: &str, enabled: bool)

    /// 获取可用 Agent 列表（前端展示用）
    pub fn list(&self) -> Vec<AgentInfo>

    /// 获取 Agent 实例（发送消息用）
    pub fn get(&self, id: &str) -> Option<&dyn AgentInterface>
}
```

### 5.2 启动流程

```rust
// lib.rs setup
let registry = AgentRegistry::new(hermes_home);

// Step 0（瞬时，同步）: 从 agents.json 加载
//   - 内置 Hermes Agent 强写入（兜底）
//   - 上次检测到的 Claude Code / OpenClaw 等
//   - 用户手动添加的 DeepSeek / GLM 等
let agents = registry.load_persisted();
app.emit("agents:updated", &agents);  // 前端立刻显示完整列表

// Step 1（后台异步，不阻塞 UI）: 重新扫描
//   - 三层检测更新 version/available 状态
//   - 发现新 Agent 追加到 agents.json
//   - 已卸载的 Agent 标记 available=false（不删除配置）
let registry_clone = registry.clone();
tauri::async_runtime::spawn(async move {
    registry_clone.background_scan().await;
    app.emit("agents:updated", &registry_clone.list());
});

app.manage(registry);
```

### 5.3 用户路径

```
场景 1: 首次使用
  → agents.json 不存在
  → load_persisted() 创建文件，写入内置 Hermes
  → background_scan() 发现 Claude Code / OpenClaw
  → 追加到 agents.json
  → 前端通知 "检测到 2 个新 Agent"

场景 2: 日常使用
  → agents.json 已有 4 个 Agent
  → load_persisted() 瞬时加载全部
  → 前端下拉框立刻显示 4 个 Agent
  → background_scan() 异步更新状态（静默）

场景 3: 用户新装了 Codex CLI
  → 打开设置 → 点"重新检测"
  → re_detect() 三层扫描 → 发现 Codex
  → 追加到 agents.json
  → 前端立刻显示 Codex 在列表中

场景 4: 用户想用 DeepSeek（云端 API）
  → 点"手动添加" → 选 DeepSeek 预设 → 填 API Key
  → add_manual() → 写入 agents.json
  → 立刻出现在下拉框中

场景 5: Agent 被卸载
  → 启动时 background_scan() 发现 Claude Code 路径不存在
  → 标记 available=false（不删除配置）
  → 前端显示 "Claude Code (未检测到)"
  → 某天用户重装 → 下次扫描恢复 available=true
```

---

## 6. 前端架构

### 6.1 设置页 — Agent 管理

```
┌─────────────────────────────────────────────────┐
│ Agent 管理                                       │
│                                                 │
│ 已检测到的 Agent:                                │
│ ┌─────────────────────────────────────────────┐ │
│ │ ● Hermes Agent (内置)            ✓ 运行中   │ │
│ │ ● Claude Code  ~/.local/bin/claude  ✓ 已检测│ │
│ │ ● OpenClaw     :18789              ✓ 运行中 │ │
│ │ ○ Codex CLI    未安装                        │ │
│ │                                              │ │
│ │ [重新检测]                                    │ │
│ └─────────────────────────────────────────────┘ │
│                                                 │
│ 默认 Agent: [Hermes Agent ▼]                    │
│ 默认模型:   [claude-sonnet-4-6 ▼]               │
└─────────────────────────────────────────────────┘
```

### 6.2 聊天页 — Agent 切换

```
┌─ 对话 1 [Hermes] ── 对话 2 [Claude] ── 对话 3 [DS] ── + ─┐
│                                                            │
│ ┌──────────────────────────────────────────────────────┐   │
│ │ 用户: 帮我写个 Python 脚本                             │   │
│ │                                                      │   │
│ │ Hermes: 好的，这是脚本...                              │   │
│ │                                                      │   │
│ │ 用户: 现在用 Claude Code 重构一下                      │   │
│ │        ↑ 自动检测到"Claude Code"关键词                 │   │
│ │        ↑ 弹出提示："要用 Claude Code 处理吗？"         │   │
│ └──────────────────────────────────────────────────────┘   │
│                                                            │
│ 当前 Agent: [Hermes Agent ▼]  输入框...         [发送]     │
└────────────────────────────────────────────────────────────┘
```

### 6.3 新前端文件

- `src/hooks/useAgentRegistry.ts` — 前端 Agent 状态管理
- `src/components/settings/AgentSettings.tsx` — 设置页 Agent 管理面板
- `src/components/chat/AgentSwitcher.tsx` — 聊天页 Agent 切换下拉

---

## 7. 与 Nexus 的关系

### 7.1 Agent 切换不影响 Nexus

```
Agent 聊天提取（有对话上下文）:
  用户点"保存到知识库"
  → Shell 用当前 active_agent 执行提取 prompt
  → Agent 返回标准 JSON Schema 的实体/关系
  → Shell 调 nexus_store(json) 入库

独立提取（无对话上下文）:
  Wiki 保存/文档上传
  → Shell 直接调 nexus_extract_from_text()
  → Nexus 内部 extract_service.py 处理
  → 不经过任何 Agent
```

Agent 切换不影响 Nexus，Nexus 不感知 Agent。

### 7.2 Nexus 也需要大模型

Nexus 的 extract_service.py 等独立服务调用 LLM API 做知识提取，需要配置 API Key。**默认直接复用 Agent 聊天模型的 API Key**，注册时一次填写两处共用。高级用户可在设置中为 Nexus 单独指定模型。详见 [Nexus 方案 §0.6](./nexus-knowledge-engine.md)。

### 7.3 ApiSetupWizard 增加 Agent 检测步骤

注册流程从现在的 1 步（填 API Key → 完成）改为 2 步：

**Step 1**: 填写 API Key（不变，Agent 聊天 + Nexus 提取共用，详见 [Nexus 方案 §0.6.1](./nexus-knowledge-engine.md#061-注册引导-ui)）

**Step 2**: 展示检测到的本地 Agent（新增，纯展示，无需用户操作）

```
┌──────────────────────────────────────────────────────────┐
│ Step 2: 检测本地 Agent                                    │
│                                                          │
│ 以下是在你电脑上发现的 AI Agent，可直接切换使用：           │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │ ● Hermes Agent (内置)                              │  │
│  │   本地服务 :18642                          ✓ 已就绪  │  │
│  │ ● Claude Code                                      │  │
│  │   ~/.local/bin/claude                   v1.0.37    │  │
│  │   检测到已安装                                ✓     │  │
│  │ ○ OpenClaw                                         │  │
│  │   未检测到安装                          可稍后手动添加 │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  检测到 2 个 Agent，随时可在设置中重新检测或手动添加        │
│                                                          │
│  [← 上一步]                                  [开始使用]   │
└──────────────────────────────────────────────────────────┘
```

**关键设计**：
- 纯展示页，无需用户操作（只读）
- 检测在 Step 1 用户填 Key 时后台异步进行，进入 Step 2 时结果已就绪
- 已检测到的显示路径 + 版本，绿色 ✓；未检测到的灰色提示 "可稍后手动添加"
- 内置 Hermes 永远显示 "已就绪"
- 点 "开始使用" → 写入 `agents.json` + `config.yaml` → 进入 AppShell
- 不影响现有 `AuthStage` 枚举，在 `ApiSetupWizard` 内部用 `WizardStep` 状态机管理

### 7.4 Agent 自动配置 Nexus 工具

切换 Agent 后，新 Agent 需要能够**主动调用 Nexus 知识图谱工具**（`nexus_map`、`nexus_search`、`nexus_detail`、`nexus_paths`、`nexus_neighbors`），才能实现"知识图谱随Agent切换无缝工作"。

不同 Agent 类型接入 Nexus 的方式不同：

| Agent 类型 | Nexus 接入方式 | 自动配置？ | 说明 |
|-----------|--------------|----------|------|
| **Hermes (内置)** | Python 端已注册 5 个 nexus_* 工具 schema | ✅ 已实现 | 无需改动 |
| **Claude Code** | MCP Server（Streamable HTTP）| ✅ 首次检测时自动 `claude mcp add` | Nexus 加 `/mcp` 端点 |
| **OpenAI 兼容** | HTTP 请求中附带 `tools` 数组 | ✅ 每次请求自动注入 | Rust 层拼接 tools JSON Schema |
| **OpenClaw** | 同 OpenAI 兼容 | ✅ 同上 | 同 OpenAI 兼容 |

#### 7.4.1 Claude Code — MCP 集成

检测到 Claude Code 时，AgentRegistry 自动执行：

```bash
claude mcp add nexus http://localhost:18643/mcp --transport streamable-http
```

只执行一次（检查 `claude mcp list` 是否已有 nexus 注册）。

Nexus 侧增加 `/mcp` 端点，将 5 个 nexus 工具暴露为 MCP tools：

```
GET  /mcp                     → MCP Server Info
POST /mcp                     → JSON-RPC (tools/list, tools/call)
```

Claude Code 收到用户消息后，自行决定是否调用 nexus 工具查询知识图谱，无需 AI-Hel2 在中间注入任何上下文。

#### 7.4.2 OpenAI 兼容 Agent — tools 注入

OpenAI 兼容 / OpenClaw / DeepSeek 等 HTTP 类 Agent，在 Rust 层每次发送 `/v1/chat/completions` 请求时，自动附带 Nexus 工具定义：

```rust
// src-tauri/src/services/agents/openai_compatible_agent.rs
fn build_nexus_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "nexus_search".into(),
            description: "搜索本地知识库中的实体和概念".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query":     {"type": "string", "description": "搜索关键词"},
                    "top_k":     {"type": "integer", "default": 5},
                    "namespace": {"type": "string", "description": "命名空间过滤"}
                },
                "required": ["query"]
            }),
        },
        // nexus_map、nexus_detail、nexus_paths、nexus_neighbors 同理
    ]
}
```

Rust 层拦截 Agent 返回的 `tool_calls` → 调用本地 Nexus API (`:18643`) → 结果注入回 `messages` → 继续生成，直到 Agent 返回最终文本回复。

#### 7.4.3 工具定义

5 个 Nexus 工具暴露给所有 Agent：

| 工具 | 用途 | 参数 |
|------|------|------|
| `nexus_search` | 关键词/语义搜索实体 | `query`, `top_k`, `namespace` |
| `nexus_map` | 获取实体邻域图谱 | `entity_id`, `depth` |
| `nexus_detail` | 获取实体完整详情 | `entity_id` |
| `nexus_paths` | 查找两实体间路径 | `from_id`, `to_id`, `max_depth` |
| `nexus_neighbors` | 获取实体邻居列表 | `entity_id`, `relation_type` |

#### 7.4.4 自动配置流程

```
Agent 检测成功
  │
  ├─ Hermes 内置:     无需操作（已有）
  ├─ Claude Code:     自动执行 claude mcp add nexus ...
  │                   标记配置完成，下次启动跳过
  ├─ OpenAI 兼容:     注册时标记 tools=nexus_tools
  │                   每次请求自动附带
  ├─ OpenClaw:        同 OpenAI 兼容
  └─ 手动添加:        同 OpenAI 兼容
```

所有 Agent 统一通过 `localhost:18643` 访问 Nexus API，无需外部网络。

#### 7.4.5 新增文件

- `src-tauri/src/services/nexus_mcp.rs` — MCP 端点实现（~100 行）
- `src-tauri/src/services/agents/nexus_tools.rs` — 5 个工具 JSON Schema 定义（~50 行）

---

## 8. 实施阶段

| Phase | 内容 | 预计 |
|-------|------|------|
| **P1 Agent 检测** | AgentDetector（三层检测）、agents.json 持久化、ApiSetupWizard 增加检测步骤 | 1.5d |
| **P2 AgentInterface** | Trait 定义、ChatEvent 统一事件流、ProviderPreset | 1d |
| **P3 Agent 实现** | HermesAgent 重构、ClaudeCodeAgent、OpenClawAgent、OpenAICompatibleAgent | 2d |
| **P4 AgentRegistry** | Registry、启动检测、lib.rs 集成、Tauri 命令暴露 | 1d |
| **P5 对话绑定** | Session.agent_id 迁移、Agent 切换时传递当前对话历史 messages | 0.5d |
| **P6 前端** | useAgentRegistry、AgentSettings、AgentSwitcher、对话胶囊条 mode 显示 | 1.5d |
| **P7 对接 Nexus** | Agent 提取接口适配、KG 存储不变 | 0.5d |
| **P8 Nexus 工具自动配置** | `/mcp` 端点、tools JSON Schema、自动注册逻辑 | 0.5d |

---

## 9. 关键设计决策

| 决策 | 理由 |
|------|------|
| Agent 检测与 Agent 实现分离 | 检测失败不影响已有 Agent 运行 |
| 统一 ChatEvent 而非每种 Agent 自定义事件 | Shell 不需要知道后面是谁 |
| 内置 Hermes 始终注册 | 兜底，确保永远有一个可用 Agent |
| CLI Agent 不走 HTTP | Claude Code 没有 HTTP 接口，spawn 子进程是唯一途径 |
| 跨 Agent 不共享上下文 | 每个对话独立绑定 Agent，不实现跨 Agent 记忆层 |
| 对话历史跟随 Session | 切换 Agent 后传当前对话 messages，Session 级隔离 |
| 文档/图片提取不经过 Agent | 走 Nexus 独立 extract_service，不绑定 Agent |
| Nexus LLM 默认复用 Agent API Key | 注册时一次填写两处共用，详见 Nexus 方案 |
| Agent 自动配置 Nexus 工具 | 检测到 Agent 后自动注册 MCP/注入 tools，用户无感知 |

---

## 10. Verification

- [ ] 启动时自动检测到系统已安装的 Claude Code 和 OpenClaw
- [ ] ApiSetupWizard Step 2 展示检测结果（路径/版本正确），点"开始使用"进入主 App
- [ ] 设置页显示所有检测到的 Agent 及其状态
- [ ] 对话 A 用 Hermes 聊天，切换到对话 B 用 Claude Code，两个对话独立运行
- [ ] 对话 A 切换 Agent 后，新 Agent 收到当前对话的 messages 历史
- [ ] Claude Code 5 分钟无输出 → 超时断开，不阻塞 UI
- [ ] Claude Code 未安装时，Agent 列表中显示为"未安装"
- [ ] Agent 检测失败不阻塞启动，内置 Hermes 始终可用
- [ ] Nexus 知识提取不受 Agent 切换影响，始终可保存到知识库
- [ ] Claude Code 首次检测后自动注册 `claude mcp add nexus`，可调用 nexus_search/nexus_map 等工具
- [ ] OpenAI 兼容 Agent 每次请求自动附带 Nexus tools，返回 tool_calls 后可正确调 Nexus API 并继续生成
