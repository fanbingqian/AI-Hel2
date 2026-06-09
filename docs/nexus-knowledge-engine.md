# Nexus 知识图谱引擎方案 v4

## 0. 架构总览

### 0.1 核心设计原则

**KG 是纯存储+查询模块，不绑定任何特定 LLM 或 Agent。Agent 通过标准接口与 KG 通信。**

```
                        Nexus Knowledge Graph
┌──────────────────────────────────────────────────────────────────┐
│                                                                  │
│   ┌─────────────┐    标准存储接口 (Rust invoke)                   │
│   │ Agent       │──→ nexus_store(raw_text, source_type, context) │
│   │ (聊天/对话)  │    Agent 只传原始数据，不负责提取               │
│   └─────────────┘                                                │
│                                                                  │
│   ┌─────────────┐                                                │
│   │ Wiki 正文    │──→ nexus_store(text, source_type)              │
│   │ 画板节点     │──→ nexus_store(json)                          │
│   │ (用户触发)    │    所有输入统一调 nexus_store 存储原始数据      │
│   └─────────────┘                                                │
│                                                                  │
│   ┌──────────────────────────────────────────┐                   │
│   │       extract_service.py (Nexus 内部)     │                   │
│   │                                          │                   │
│   │  监听新入库数据 → 调 LLM API 做知识提取    │                   │
│   │  --mode text / document / image           │                   │
│   │  → 实体/关系 JSON → 回写 KG 数据库         │                   │
│   └────────────┬─────────────────────────────┘                   │
│                │                                                  │
│   ┌────────────▼─────────────────────────────┐                   │
│   │          SQLite (本地存储)                │                   │
│   │  cache_entities / cache_relations         │                   │
│   │  cache_synthesis / cache_ontology         │                   │
│   │  cache_content_index / cache_extraction_feedback       │                   │
│   └────────────┬─────────────────────────────┘                   │
│                │                                                  │
│   ┌────────────▼─────────────────────────────┐                   │
│   │  Heimdall (图查询 + 树查询 API)           │                   │
│   └──────────────────────────────────────────┘                   │
│                                                                  │
│   ┌──────────┐ ┌──────────┐ ┌──────────┐                    │
│   │ 图谱视图  │ │ 知识编辑  │ │ 设置     │                    │
│   └──────────┘ └──────────┘ └──────────┘                    │
└──────────────────────────────────────────────────────────────────┘
```

### 0.2 设计原则：Agent 只存不提取

```
Agent 的职责:                       Nexus 的职责:
  对话 / 文件 / 图片                  收到原始数据后
       │                                │
       ▼                                ▼
  nexus_store(原始数据)            extract_service.py 调 LLM 提取
  (不管提取，只管存)                 实体/关系 JSON → 写入 KG
```

**为什么 Agent 不负责提取**：
- Agent 不知道提取规则——提取 prompt、过滤逻辑、置信度模型都是 Nexus 的实现细节
- Agent 更换时零改动——不管 Hermes / Claude Code / DeepSeek，都只调同一个 `nexus_store` API
- 提取质量由 Nexus 统一控制——调整提取策略时只改 extract_service.py，不影响任何 Agent

**对话中的数据 vs 知识库页的数据**：

| 维度 | 对话中触发 | 知识库页触发 |
|------|---------|---------|
| 示例 | 用户在聊天中传图片、发消息后点"保存" | 用户在知识库页上传文档、保存 Wiki |
| 谁调 nexus_store | Agent（在对话流程中） | 前端直接调 |
| 有对话上下文吗 | **有** — nexus_store 可附带历史消息作参考 | **无** — 纯文本/文件 |
| 谁提取 | extract_service.py | extract_service.py |
| 提取质量 | 更高（上下文帮助消歧） | 正常 |

**关键**：两条路径最终都走 `extract_service.py` 提取。区别只在调 `nexus_store` 时是否附带对话上下文，上下文作为提取 prompt 的补充信息。

### 0.3 Agent 与 KG 的标准接口

```
┌─────────────────────────────────────────────────┐
│  Agent 只需要做一件事:                            │
│                                                  │
│  当用户触发"保存到知识库":                        │
│  → Agent 调 Rust invoke("nexus_store", {        │
│        text: "对话中的原始文本/文件内容",          │
│        source_type: "chat",                     │
│        context: [...对话上下文消息...]  // 可选    │
│    })                                           │
│                                                  │
│  Agent 更换时:                                   │
│  - 新 Agent 只需知道 nexus_store 的调用方式       │
│  - 不需要理解提取 prompt、过滤规则、入库流程       │
│  - KG 模块完全不受影响                            │
│                                                  │
│  提取由 Nexus 内部异步完成:                       │
│  nexus_store 写入 → extract_service.py 自动处理    │
└─────────────────────────────────────────────────┘
```

### 0.4 标准提取 Prompt 模板（extract_service.py 内部）

extract_service.py 收到原始文本后，按此模板调 LLM 提取：

```
## 角色
你是知识筛选器。从以下内容中提取值得长期保存的知识。

## 对话上下文（可选，仅 chat 类来源附带）
{conversation_context}

## 待提取内容
{raw_text}

## 规则
1. 只提取具有长期知识价值的实体：概念、工具、项目、人物、术语
2. 忽略：问候语、临时指代、通用词、纯格式标记
3. 如有对话上下文，利用它理解指代和省略

## 输出 JSON
{
  "entities": [
    {
      "name": "实体名",
      "type": "自由描述 (如 tool/concept/person/project)",
      "namespace": "语义领域 (如 技术/开发工具)",
      "description": "一句话描述",
      "confidence": 0.0
    }
  ],
  "relations": [
    {
      "from": "实体A名",
      "type": "关系描述 (如 uses/depends_on/creates)",
      "to": "实体B名",
      "confidence": 0.0
    }
  ]
}

## 置信度参考
0.9 - 核心概念、用户明确标记为重要
0.7 - 具体名称、清晰定义
0.5 - 有信息量但不够独立
0.3 - 边缘提及（通常不输出）
```

### 0.5 标准存储接口

```rust
// Agent 或前端调用：存入原始数据，触发异步提取
#[tauri::command]
nexus_store(
    text: String,              // 原始文本内容
    source_type: String,       // "chat" | "wiki" | "upload_doc" | "upload_image" | "canvas" | "agent_memory"
    source_path: Option<String>, // 来源文件路径（wiki 有，chat 无）
    context: Option<String>,   // 对话上下文（仅 chat 类来源，JSON 格式的消息数组）
) -> Result<NexusStoreResult, String>
// → Rust 写入 cache_content_index（extracted_at=NULL 标记为待处理）
// → source_type="canvas" 时直接映射节点/连线为实体/关系，不走 LLM
// → 其他类型：extract_service.py 异步轮询 → 提取 → 回写 extracted_at 时间戳

// 外部文件变化触发提取（FileWatcher 路径）
#[tauri::command]
nexus_extract_from_file(
    file_path: String,         // 文件路径
    source_type: String,       // "wiki" | "upload_doc" | "upload_image"
) -> Result<NexusStoreResult, String>
// → 内部计算 hash 去重 → spawn extract_service.py --mode X → 入库

// 全量重建索引（用户手动触发）
#[tauri::command]
nexus_reindex_all() -> Result<NexusReindexResult, String>
// → 遍历 heimdall/wiki/** + sessions/* → 逐个调 extract_service.py

// 测试 Nexus LLM 连通性
#[tauri::command]
check_nexus_llm_connection(config: NexusLlmConfig) -> Result<NexusLlmStatus, String>

// NexusLlmConfig 结构（用于测试连接和自定义模式配置）
struct NexusLlmConfig {
    provider: String,    // "anthropic" | "openai" | "deepseek" | "custom"
    model: String,       // "claude-sonnet-4-6" | "deepseek-chat" | ...
    api_key: String,
    base_url: Option<String>,  // 自定义 endpoint，None 则用 provider 默认
}

// 返回类型定义
struct NexusStoreResult {
    entity_count: u32,       // 本次提取的实体数
    relation_count: u32,     // 本次提取的关系数
    skipped: bool,           // content_hash 匹配，跳过提取
}

struct NexusReindexResult {
    files_processed: u32,
    entities_total: u32,
    relations_total: u32,
    skipped: u32,            // hash 匹配跳过的文件数
    errors: Vec<String>,
}

struct NexusLlmStatus {
    ok: bool,
    model: String,
    latency_ms: u64,
    error: Option<String>,
}
```

---

### 0.6 Nexus LLM 配置模型

Nexus 的知识提取（extract_service.py 等）依赖 LLM API。配置支持两种模式：

```
config.yaml 新增:

nexus:
  llm_mode: "follow_agent"    # "follow_agent" | "custom"
  # --- 以下仅在 custom 模式下有效 ---
  llm_provider: "deepseek"
  llm_model: "deepseek-chat"
  llm_api_key: "sk-xxx"
  llm_base_url: "https://api.deepseek.com/v1"
```

**模式 1: `follow_agent`（默认，推荐）**

extract_service.py 自动读取 Agent 聊天模型的 API Key / provider / model，直接复用。

**模式 2: `custom`（高级用户）**

用户可为每次知识提取指定不同的模型：便宜的做日常提取，贵的做深度合成。

**启动时验证**：首次使用引导中检测 Nexus LLM 连通性，确保用户进去就能用。测试命令 `check_nexus_llm_connection` 定义见 §0.5。

#### 0.6.1 注册引导 UI

首次注册时，ApiSetupWizard 从 1 步改为 2 步。同一套 API Key 同时用于 Agent 聊天和 Nexus 知识提取：

```
┌──────────────────────────────────────────────────────────┐
│ Step 1: 配置大模型 API                                    │
│                                                          │
│ 填写 API Key 即可使用。同一个 Key 同时用于：               │
│ ● Agent 聊天 — Hermes / Claude Code 等 AI 对话           │
│ ● 知识提取 — Nexus 将对话、文档自动整理为知识图谱          │
│                                                          │
│  ┌─ Anthropic  ●  Claude 系列    [sk-ant-xxx...]  已配置 ─┐│
│  ┌─ OpenAI     ○  GPT 系列       [粘贴 API Key...] 未配置 ─┐│
│  ┌─ DeepSeek   ●  V3 / R1        [sk-xxx...]      已配置 ─┐│
│  ...                                                      │
│                                                          │
│  已配置 2 个模型                                          │
│                                                          │
│  [跳过，稍后配置]                              [下一步 →]  │
└──────────────────────────────────────────────────────────┘
```

Step 1 变化点：
- 描述文字新增 "同一个 Key 同时用于 Agent 聊天 + Nexus 知识提取"
- "开始使用" 按钮改为 "下一步 →"
- 至少填一个 Key 才能进下一步

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

Step 2 检测在 Step 1 用户填 Key 时后台异步进行，进入 Step 2 时结果已就绪。点 "开始使用" → 写入 `config.yaml` + `agents.json` → 进入主 App。

**实现方式**：不影响现有 `AuthStage` 枚举（`"api_setup"` 不变），在 `ApiSetupWizard` 内部用 `wizardStep` state 管理两步切换。详见 [multi-agent-switching.md §7.3](./multi-agent-switching.md#73-apisetupwizard-增加-agent-检测步骤)。

---

## 1. 提取策略：宁少勿滥

### 1.1 触发源与执行者

| 触发源 | 触发方式 | 谁调 nexus_store | 谁提取 | 上限 |
|--------|---------|--------|--------|------|
| 聊天保存 | 用户点击"保存到知识库" | Agent | extract_service.py (有上下文) | 10 实体 |
| 对话中上传图片/文件 | 用户在聊天中传文件 → 点"保存" | Agent | extract_service.py (有上下文) | 10 实体 |
| Wiki [[wikilink]] | 用户写链接标记 | 前端 | Regex 快速通道（不走 LLM） | 不限 |
| Wiki 正文 | 用户保存 Wiki 文件 / 外部编辑器修改 | 前端 / FileWatcher | extract_service.py --mode text | 10 实体 |
| 画板节点+连线 | 用户创建/修改节点连线 | 前端 | 直接映射（不走 LLM） | 不限 |
| 知识库页上传文档 | 用户在知识库页面上传 | 前端 | extract_service.py --mode document | 20 实体 |
| 知识库页上传图片 | 用户在知识库页面上传 | 前端 | extract_service.py --mode image | 3 实体 |
| Agent 记忆 | 手动触发（设置页按钮）/ 全量备份时自动 | 前端 | extract_service.py --mode text | 10 实体/session |

**关键**：所有需要 LLM 提取的路径最终都走 `extract_service.py`。Agent 只管调 `nexus_store()` 存原始数据，提取是 Nexus 内部的事。

### 1.2 [[wikilink]] 实体的 namespace 处理

[[wikilink]] 走 regex 快速通道，没有 LLM 分配 namespace。处理方式：

```
1. 优先：如果该 Wiki 文件已通过 LLM 提取过正文 → [[wikilink]] 实体继承该文件正文提取结果中的主导 namespace
2. 次选：从该 Wiki 文件的目录路径推断（如 wiki/技术/xxx.md → "技术"）
3. 兜底：namespace = "未分类"
```

### 1.3 入库前过滤

```
1. confidence < 0.4 → 丢弃
2. name 在 STOP_WORDS → 丢弃
3. Levenshtein 相似度 > 85% 且 entity_type 相同 → 合并（保留较老实体，合并关系，取最高置信度）
4. entity_type 不同但 name 相似 > 85% → 仍合并，entity_type 保留较老实体的 entity_type，记录日志
5. Levenshtein 相似度 60-85% → 写入 cache_pending_merge 待确认表
6. name 长度 < 2 或 > 60 → 丢弃
```

STOP_WORDS 定义在 `extract_service.py` 中，硬编码常见中英文无意义词：

```python
# extract_service.py
STOP_WORDS = {
    # 中文通用
    "东西", "事情", "这个", "那个", "它们", "我们", "他们", "她们",
    "什么", "怎么", "哪里", "这里", "那里", "因为", "所以", "但是",
    "如果", "虽然", "可以", "需要", "应该", "能够", "可能", "已经",
    "没有", "知道", "觉得", "认为", "使用", "通过", "进行", "其他",
    "一些", "一下", "上面", "下面", "前面", "后面", "左右", "等等",
    "一个", "一种", "很多", "一般", "基本", "全部",
    # 英文通用
    "the", "a", "an", "this", "that", "these", "those", "it", "they",
    "he", "she", "we", "you", "i", "is", "are", "was", "were", "been",
    "being", "have", "has", "had", "do", "does", "did", "will", "would",
    "could", "should", "may", "might", "can", "shall", "to", "of", "in",
    "for", "on", "with", "at", "by", "from", "as", "into", "through",
    "during", "before", "after", "above", "below", "between", "under",
    "again", "further", "then", "once", "here", "there", "when",
    "where", "why", "how", "all", "both", "each", "few", "more",
    "most", "other", "some", "such", "no", "nor", "not", "only",
    "own", "same", "so", "than", "too", "very",
}
```

用户可通过 `~/.ai-hel2/stop_words.txt`（每行一个词）追加自定义停用词，extract_service.py 启动时合并加载。

### 1.4 错误处理与降级

| 错误场景 | 处理 |
|---------|------|
| LLM 超时（30s） | 本次跳过提取，日志记录，返回空结果不崩溃 |
| LLM 返回非法 JSON | 尝试从响应中提取 JSON 子串，失败则跳过 |
| API Key 失效 | 返回明确错误给前端："KG 模型配置无效，请检查设置" |
| 模型不可用 | 前端提示用户切换 KG 模型或检查网络 |

---

## 2. 置信度模型

> **术语约定**：文档中 `confidence` 指 LLM 输出的原始置信度（§0.4 JSON 字段），经多源确认修正后写入 DB 列 `llm_confidence`。§1.3 入库前过滤使用原始 `confidence`（此时尚未写入 DB）；§7.1 渲染阈值、§9 衰减规则以 `llm_confidence` 为准。

- **置信度由 LLM 根据语义质量打分**，不是公式计算
- Prompt 中给出锚点参考（§0.4）
- **多源确认**：同一实体被 ≥2 个**不同类型的来源**提取 → llm_confidence +0.1（上限 0.95）
  - "不同类型来源"定义：chat / wiki / canvas / upload_doc / agent_memory 中任意两种
  - wikilink 和 upload_image 不算入多源确认（wikilink 是 regex 通道不涉及 LLM 语义提取，upload_image 实体数量少）
  - 同一文件的 [[wikilink]] + 正文 LLM 提取 ≠ 两个来源（属于同一文件）
  - 不同文件提取到同一实体 → 算多源
- `source_count` 字段记录被提取次数，用于辅助判断

---

## 3. Namespace：LLM 语义分配

### 3.1 从文件路径 → 语义领域

```
旧: wiki/subdir/file.md → namespace = "subdir"
新: LLM 提取时判断每个实体属于什么语义领域

层级用 "/" 分隔:
  "技术/开发工具"
  "设计/UI框架"
  "项目管理/方法论"
```

### 3.2 自演化

- 不预定义 namespace 列表，LLM 自由创建
- 本体引擎周期性分析 namespace 分布，文本相似度聚类，建议合并相近分类
- 用户可在编辑界面手动调整实体归属

---

## 4. 知识合成引擎

### 4.1 触发时机

- 每次实体/关系入库后，5 秒 debounce 攒批处理
- 启动时也运行一次全量检查（仅检查自上次运行以来新增的实体）

### 4.2 合成规则

| 规则 | 条件 | 输出 | 置信度 |
|------|------|------|--------|
| 共享邻居 | A→X←B，X 度≥3（X 是枢纽节点），A-B 无直接边 | A—B `related_to` | 0.25 |
| 跨文档共现 | A 和 B 在 ≥3 个不同 `source_path` 中共现 | A—B `co_occurs` | 0.2 + N×0.05 |
| 类型模式发现 | 某 entity_type 的实体 80% 通过同一 relation_type 连接 | 存入 cache_ontology | — |

合成结果存入 `cache_synthesis`，边标记为 `inferred`，渲染为灰色虚线。

**去重策略**：合成前检查 `cache_relations`，若已存在相同 `(from_entity, to_entity, relation_type)` → 跳过合成（保留直接边）。若关系类型不同 → 新增独立 inferred 边。

---

## 5. 反馈闭环

### 5.1 信号定义

| 行为 | 信号 | 权重 | 触发条件 |
|------|------|------|---------|
| 隐藏实体 | 负反馈 | -0.3 | 用户点击隐藏 |
| 删除实体 | 强负反馈 | -0.5 | 用户确认删除 |
| 查看/聚焦 ≥3 次 | 正反馈 | +0.1 | 累计 3 次后触发一次 |
| 手动提升置信度 | 强正反馈 | +0.3 | 用户在编辑器中调高 |
| 链接到画板 | 正反馈 | +0.2 | 实体被拖入画板 |
| 搜索命中并点击 | 弱正反馈 | +0.05 | 搜索后点击结果 |

### 5.2 生效机制

```
每 15 条全局反馈 → 生成用户偏好摘要 → 注入提取 prompt:

"用户偏好摘要:
 用户倾向于保留: [具体工具名], [项目名], [技术概念]
 用户倾向于忽略: [泛泛术语], [临时提及], [格式碎片]
 最近被隐藏的示例: ['xxx项目第3次会议', '一个例子']
 最近被确认的示例: ['AutoCAD MCP', 'sherpa-onnx']"
```

### 5.3 自动隐藏

- 实体 `feedback_score < -0.8` → 自动 `hidden=true`
- 实体 `feedback_score < -1.5` → 标记待删除（用户确认后删）

---

## 6. 自演化本体

### 6.1 类型不预定义

- 不限制 entity_type / relation_type / namespace 的取值
- LLM 自由输出，新类型自动入库
- 新类型自动分配颜色：从 16 色调色板按使用顺序轮转

### 6.2 类型收敛

**触发**：应用启动时检查，距上次分析 > 7 天则运行

**流程**：
1. 统计所有 entity_type 的使用频率
2. 对低频类型（使用 < 3 次）做文本相似度聚类（Levenshtein + 中文同义词）
3. 发现 "tool" ≈ "工具" ≈ "软件工具" → 生成合并建议 → 存入 `cache_ontology`
4. 前端展示合并建议，用户确认 → 批量更新实体 entity_type

---

## 7. UI 设计

### 7.1 图谱视图（Sphere）

**节点**：
| 属性 | 规则 |
|------|------|
| 半径 | `3 + log2(degree+1)×2`，min 3px，max 12px |
| 颜色 | 按 entity_type（已知类型有预设，新类型轮转分配 16 色调色板） |
| 透明度 | confidence ≥0.7→1.0 / 0.4-0.7→0.6 / <0.4→不渲染 |
| 标签 | degree ≥3 始终显示，<3 hover 显示 |
| 选中 | 放大 1.4x，描边高亮 |

**边**：
| 属性 | 规则 |
|------|------|
| 颜色 | #8b95a5 灰色 |
| 宽度 | 1px 统一 |
| 样式 | 直接/LLM 提取 → solid / inferred → dashed |
| 合并边 | 同一实体对多条关系 → 单线，tooltip 列出所有关系类型 |

**自适应阈值**：
```
总实体(含hidden) < 200: 显示 confidence ≥ 0.3
200-500: 显示 confidence ≥ 0.4
500-1000: 显示 confidence ≥ 0.5
1000+: 显示 confidence ≥ 0.6
用户可切换"显示全部"覆盖。阈值变化 300ms 过渡。
```

**语义缩放**：
| 级别 | 触发 | 展示 |
|------|------|------|
| L0 概览 | 无对话、无选中 | Top 50 高 degree 实体 + 边 |
| L1 上下文 | 有活跃对话 | 对话提及实体 + 1-hop 邻居 |
| L2 聚焦 | 点击节点/搜索 | 选中实体 + 2-hop，其他 fade(0.08) |

### 7.2 知识编辑页

当前 `KnowledgeEditor` 页面（TabBar "知识编辑"），包含两个 Tab：

**文档列表**：DocTree 文件树 + MilkdownEditor。上传入口在 DocTree 标题旁 `+` 按钮 + 页面 body 拖拽上传。Wiki 文件变更通过 §11.2 的 FileWatcher 实时同步。

**实体浏览**：EntityBrowser，按 namespace 层级分组折叠。

- 默认平铺列表，可按 namespace / source_type / confidence 分组、搜索、筛选
- 点击实体 → 展开详情面板

详情面板内支持：

| 功能 | 说明 |
|------|------|
| 实体编辑 | 修改 name / type / namespace / description / confidence |
| 关系编辑 | 增删关系、修改 relation_type |
| 反馈操作 | 隐藏/显示/删除/提升置信度/链接画板——记录到 cache_extraction_feedback 表 |
| 待确认合并 | 查看 `cache_pending_merge` 表，确认合并或忽略 |
| 类型管理 | 查看本体收敛建议，确认合并类型或忽略 |
| 批量操作 | 按 namespace / source_type / confidence 筛选，批量隐藏/删除/改 namespace |

图谱中点击节点 → 跳转到知识编辑页实体详情。

### 7.3 设置（Settings）

在设置页新增 "知识图谱" 导航项，独立于 "大模型配置"（Agent 聊天专用）：

```
┌─ 设置 ────────────────────────────────────────────────┐
│ 账户信息                                               │
│ 大模型配置    ← Agent 聊天模型                          │
│ 知识图谱      ← 新增：Nexus LLM + 知识库管理             │
│ 网关配置                                               │
│ ...                                                   │
└───────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────┐
│ 知识图谱 (Nexus)                                      │
│                                                      │
│ ── 大模型配置 ──                                      │
│                                                      │
│ 知识提取使用的模型：Nexus 从对话、文档中自动提取知识实体 │
│                                                      │
│  ○ 跟随 Agent (推荐)                                  │
│    直接复用上方"大模型配置"中已保存的 API Key 和模型     │
│    当前将使用: DeepSeek V3 (deepseek-v4-pro)          │
│                                                      │
│  ○ 自定义                                              │
│    为知识提取单独指定模型（适合用便宜模型做日常提取）     │
│                                                      │
│    ┌─ 自定义时展开 ──────────────────────────────┐    │
│    │ 提供商: [DeepSeek ▼]                         │    │
│    │ 模型:   [deepseek-chat]                      │    │
│    │ Base URL: [https://api.deepseek.com/v1]      │    │
│    │ API Key:  [sk-xxx...]           [测试连接]    │    │
│    │                                              │    │
│    │ 连接状态: ✓ 连通 (deepseek-chat, 230ms)       │    │
│    └──────────────────────────────────────────────┘    │
│                                                      │
│ ── 显示偏好 ──                                        │
│                                                      │
│ 默认显示阈值: [跟随自适应 ▼]  或自定义 ≥ [0.4]          │
│ 显示 inferred 边:  [是] / [否]                         │
│ 显示 hidden 实体: [否] / [是]                          │
│                                                      │
│ ── 知识库管理 ──                                      │
│                                                      │
│ [提取 Agent 记忆到知识库]  [重新索引全部文件]           │
│ [导出知识库]              [导入知识库]                 │
│                                                      │
│ ── 反馈管理 ──                                        │
│                                                      │
│ 已记录 127 条反馈  [查看偏好摘要]  [清除所有反馈]       │
└──────────────────────────────────────────────────────┘
```

**交互逻辑**：

| 操作 | 行为 |
|------|------|
| 选中"跟随 Agent" | 下方显示当前 Agent 使用的模型名，灰色文字不可编辑 |
| 切换到"自定义" | 展开表单，自动预填 Agent 配置作为初始值 |
| 点击"测试连接" | 调用 `check_nexus_llm_connection` → 显示连通状态 + 延迟 |
| 保存 | 写入 `config.yaml` 的 `nexus:` 段 |

**实现**：设置页左侧导航新增 `knowledge` 枚举值 + `KnowledgeSection` 组件（`src/components/settings/SettingsPage.tsx`）。

---

## 8. 数据库变更

### 8.1 cache_entities — 新增字段 + 迁移

```sql
-- 新增字段
ALTER TABLE cache_entities ADD COLUMN source_type TEXT DEFAULT 'unknown';
-- 'chat' | 'wikilink' | 'wiki' | 'canvas' |
--  'upload_doc' | 'upload_image' | 'agent_memory' | 'synthesis'

ALTER TABLE cache_entities ADD COLUMN llm_confidence REAL;
-- extract_service.py 将 LLM 返回 JSON 中的 "confidence" 字段映射到此列
-- 旧数据的 confidence 值直接复制到 llm_confidence

ALTER TABLE cache_entities ADD COLUMN source_count INTEGER DEFAULT 1;

ALTER TABLE cache_entities ADD COLUMN namespace TEXT DEFAULT '未分类';

ALTER TABLE cache_entities ADD COLUMN content_hash TEXT;

ALTER TABLE cache_entities ADD COLUMN feedback_score REAL DEFAULT 0.0;

-- 现有 hidden 列保留不变，不新增 status 字段（避免冲突）
-- hidden=1 等同于旧方案中的 status='hidden'
```

### 8.2 迁移方式

```
src-tauri/
└── migrations/
    ├── 001_init.sql              # 现有初始结构
    ├── 002_nexus_schema.sql      # 本次新增: cache_entities 新字段 + 新表
    └── ...
```

应用启动时 Rust 读取 `cache_meta` 表的 `schema_version` 字段，顺序执行未跑过的 SQL 文件。版本号从 1 开始递增。

### 8.3 旧数据迁移

```sql
-- 有 [[wikilink]] 特征的标记为 'wikilink'
-- 其余按 source_file 推断：
--   source_file LIKE 'chat%' → 'chat'
--   source_file LIKE '%.md' → 'wiki'
--   source_file LIKE 'source:%' → 'chat'

-- namespace 回填：
--   从 source_file 路径提取（如 wiki/技术/xxx.md → "技术"）
--   无可提取路径 → "未分类"

-- llm_confidence 回填：
--   现有 confidence 字段的值直接复制
--   source_count 默认 1
```

### 8.4 新增表

```sql
CREATE TABLE cache_synthesis (
  id TEXT PRIMARY KEY,
  entity_a_id TEXT NOT NULL,
  entity_b_id TEXT NOT NULL,
  method TEXT NOT NULL,  -- 'shared_neighbor' | 'co_occurrence' | 'type_pattern'
  inferred_relation_type TEXT NOT NULL DEFAULT 'related_to',
  confidence REAL NOT NULL DEFAULT 0.25,
  reasoning TEXT,  -- 推理描述："共享枢纽节点 X(度=N)"
  created_at TEXT NOT NULL,
  FOREIGN KEY (entity_a_id) REFERENCES cache_entities(id),
  FOREIGN KEY (entity_b_id) REFERENCES cache_entities(id)
);

CREATE TABLE cache_ontology (
  id TEXT PRIMARY KEY,
  category TEXT NOT NULL,  -- 'entity_type' | 'relation_type' | 'namespace'
  type_name TEXT NOT NULL,
  usage_count INTEGER DEFAULT 1,
  canonical_suggestion TEXT,  -- 建议合并到的标准名称
  similar_types TEXT,  -- JSON: ["tool","工具","软件工具"]
  status TEXT DEFAULT 'pending',  -- 'pending' | 'confirmed' | 'ignored'
  last_analyzed TEXT NOT NULL
);

CREATE TABLE cache_content_index (
  source_path TEXT PRIMARY KEY,
  source_type TEXT NOT NULL,
  content_hash TEXT NOT NULL,  -- SHA256
  extracted_at TEXT,
  entity_count INTEGER DEFAULT 0
);

CREATE TABLE cache_extraction_feedback (
  id TEXT PRIMARY KEY,
  entity_id TEXT,
  entity_name TEXT,
  action TEXT NOT NULL,  -- 'hide'|'show'|'delete'|'view'|'boost'|'canvas_link'|'search_hit'
  score REAL NOT NULL,
  source_type TEXT,
  entity_type TEXT,
  created_at TEXT NOT NULL
);

CREATE INDEX idx_feedback_created ON cache_extraction_feedback(created_at);

CREATE TABLE cache_pending_merge (
  id TEXT PRIMARY KEY,
  entity_a_id TEXT NOT NULL,
  entity_b_id TEXT NOT NULL,
  entity_a_name TEXT NOT NULL,
  entity_b_name TEXT NOT NULL,
  similarity REAL NOT NULL,  -- Levenshtein 相似度
  status TEXT DEFAULT 'pending',  -- 'pending' | 'confirmed' | 'ignored'
  created_at TEXT NOT NULL
);
```

### 8.5 保留现有表

- `cache_entity_scores` — 保留，继续记录 view/focus/reference 统计
- `cache_operations_log` — 保留
- `cache_pending_sync` — 保留
- `cache_relations` — 保留，现有关系表（from_entity / to_entity / relation_type / confidence）
- `cache_meta` — 保留，记录元数据（`schema_version` 用于迁移版本管理，key-value 结构）

---

## 9. 衰减清理

触发时机：应用启动时 + `scan_wiki_directory` 执行时

```
hidden=true 且 updated_at > 30 天 → 物理删除实体及其关系
hidden=true 且 updated_at > 7 天 → 保持不变（等待 30 天）
degree=0 且 confidence<0.5 且 created_at > 7 天 → 自动 hidden=true
```

---

## 10. 实施阶段

| Phase | 内容 | 预计 |
|-------|------|------|
| **P1 清理止血** | 删 Pattern3/5b/doc-anchor、修 Pattern5、Schema 迁移、旧数据回填、全局去重 | 1-2d |
| **P2 标准接口** | `nexus_store` / `nexus_extract_from_file` / `nexus_reindex_all` 命令、标准 JSON Schema 校验、source_type='agent_memory' 支持 | 1-2d |
| **P3 提取服务** | extract_service.py（--mode text/document/image）、Nexus LLM 配置 UI（注册引导 + 设置页） | 2-3d |
| **P4 合成+本体** | 合成引擎(3规则)、cache_synthesis、cache_ontology、类型收敛 | 2-3d |
| **P5 UI 重构** | 置信度驱动渲染、EntityBrowser(namespace分组折叠)、自适应阈值、语义缩放、搜索过滤、文档/图片上传入口 | 2-3d |
| **P6 编辑+反馈** | 知识编辑器重构、待确认合并、类型管理、反馈闭环、偏好摘要、衰减清理 | 2-3d |
| **P7 备份恢复** | 全量备份打包、manifest.json、恢复校验、定时自动备份 | 1d |

---

## 11. 文件布局与实时同步

### 11.1 实际文件结构

Nexus 知识库的所有文件与 Agent 记忆文件都在 `~/.ai-hel2/` 同一根目录下：

```
~/.ai-hel2/
├── heimdall/                          # ← 知识库核心目录
│   ├── heimdall.db                     #    图数据库（Heimdall 引擎）
│   ├── heimdall.db-shm / -wal          #    SQLite WAL 实时写入
│   └── wiki/                           #    Wiki Markdown 源文件
│       ├── 欢迎使用 AI-Hel2.md
│       ├── 日记/
│       ├── 画板/
│       ├── 笔记/
│       │   └── 示例笔记.md
│       └── 项目/
│
├── knowledge_cache.db                  # ← KG 知识缓存（实体/关系/合成/本体）
├── knowledge_cache.db-shm / -wal       #    SQLite WAL（当前 WAL 4MB+，活跃写入）
│
├── sessions/                           # ← Agent 记忆文件（新增提取源）
│   ├── session_api-{id}.json           #    完整会话记录（含 messages 数组）
│   └── request_dump_api-{id}_{ts}.json #    单次请求/错误转储
│
├── memories/                           # ← 持久化记忆（当前为空，预留）
├── state.db                            #    应用状态
├── response_store.db                   #    响应缓存
├── kanban.db                           #    看板数据
└── config.yaml / auth.json / ...       #    配置文件
```

**关键结论**：

| 问题 | 答案 |
|------|------|
| 知识库文件和 KG 数据库在同一位置吗？ | **是** — 都在 `~/.ai-hel2/` 下。Wiki 源文件在 `heimdall/wiki/`，KG 缓存数据库在根目录 `knowledge_cache.db`，图数据库在 `heimdall/heimdall.db` |
| 是实时同步的吗？ | **准实时** — 见 §11.2 的 FileWatcher + 3s debounce 机制 |
| 可以随时提取吗？ | **可以** — 用户主动操作（保存/上传）立刻触发提取；外部文件修改 3 秒后自动触发；也可手动触发全量 re-index |
| 提取时能备份 Agent 记忆吗？ | **能** — `sessions/` 目录中的会话 JSON 可作为新的提取源，见 §11.4 |

### 11.2 实时同步机制

```
外部编辑器修改 Wiki 文件
  │
  ▼
FileWatcher 检测到文件变更（heimdall/wiki/**/*.md）
  │
  ├─ 3 秒 debounce（合并连续保存事件）
  │
  ├─ 计算文件 SHA256 → 对比 cache_content_index.content_hash
  │     ├─ hash 相同 → 跳过（内容未变，只是 touch）
  │     └─ hash 不同 → 触发提取
  │
  ▼
extract_service.py 调用 LLM 提取实体/关系
  │
  ▼
Rust nexus_extract_from_file() → 写入 knowledge_cache.db
  │
  ▼
heimdall.db 通过 Heimdall API 同步可查询的图结构（Heimdall 直接读取 knowledge_cache.db 中的 cache_entities / cache_relations，构建内存图索引供图谱视图查询，无需数据复制）
```

**同步延迟**：

| 场景 | 延迟 |
|------|------|
| 用户在 AI-Hel2 内保存 Wiki | 即时（保存 → 直接调提取） |
| 用户用外部编辑器修改 .md 文件 | ≤ 3 秒（FileWatcher debounce） |
| 用户点"保存到知识库" | 即时（Agent 调 nexus_store → extract_service.py 异步提取） |
| 上传文档/图片 | 即时（用户主动操作） |

### 11.3 重复提取防护（content_hash 去重）

```sql
-- cache_content_index 表记录了每个文件的提取状态
-- 同一个文件内容不同 → 重新提取
-- 同一个文件内容相同 → 跳过（但更新 extracted_at 时间戳供审计）

-- 流程：
-- 1. 计算文件 SHA256
-- 2. SELECT content_hash FROM cache_content_index WHERE source_path = ?
-- 3. hash 相同 → skip; hash 不同 → extract → UPDATE content_hash, extracted_at
```

### 11.4 Agent 记忆作为新提取源

**当前 sessions/ 目录中的数据**：

| 文件类型 | 格式 | 内容 | 单文件大小 |
|---------|------|------|-----------|
| `session_api-{id}.json` | JSON | `{session_id, model, base_url, messages[{role,content}], system_prompt, tools}` | ~84KB |
| `request_dump_api-{id}_{ts}.json` | JSON | `{timestamp, session_id, reason, request, error}` | ~82-90KB |

**sessions/ 的数据特点**：
- 每个 session JSON 包含完整的多轮对话 messages 数组
- 用户与 Agent 的所有交互历史都在其中
- 当前有 6 个 session + 21 个 request_dump，总计约 2.3MB
- 每次对话都会更新对应的 session JSON

**将 Agent 记忆接入提取管线**：

Agent 记忆提取是**独立于对话的批量操作**，属于"非聊天提取"，走 `extract_service.py` 管道：

```
用户触发"提取 Agent 记忆"
  │
  ▼
Rust 扫描 sessions/*.json → 按 session_id 去重 → 取最新版本
  │
  ▼
对每个 session:
  1. 拼接所有 messages 为文本（user/assistant 交替）
  2. 计算 content_hash → 对比 cache_content_index
  3. hash 相同 → 跳过；hash 不同 → 送入 extract_service.py
  │
  ▼
extract_service.py → 标准提取 prompt → LLM → 实体/关系 JSON
  │
  ▼
入库 knowledge_cache.db（source_type = "agent_memory"）
```

**提取源类型扩展**：

```sql
-- cache_entities.source_type 完整枚举值
-- 'chat' | 'wikilink' | 'wiki' | 'canvas' | 'upload_doc' | 'upload_image' | 'agent_memory' | 'synthesis'
```

**触发时机**：

| 触发方式 | 说明 |
|---------|------|
| 用户手动触发 | 设置页"提取 Agent 记忆到知识库"按钮 |
| 全量备份时触发 | 备份前自动提取一轮，确保 KG 包含最新会话知识 |

Agent 有自己的 sessions/ 记忆机制，KG 提取由用户主动决定，不做自动 debounce 提取。

### 11.5 全量备份与恢复

**备份范围**：`~/.ai-hel2/` 下所有数据均可打包备份：

```
备份包结构:
  ai-hel2-backup-20260529.zip
  ├── heimdall/           # Wiki 源文件 + 图数据库
  ├── knowledge_cache.db  # KG 知识缓存（提取结果）
  ├── sessions/           # Agent 记忆原始 JSON
  ├── memories/           # 持久化记忆
  ├── config.yaml         # 配置
  └── manifest.json       # 备份元信息（时间、版本、文件清单、hash）
```

**备份触发**：
- 用户手动：设置页"导出知识库" → 打包为 zip → 用户选择保存路径
- 定时自动：可配置周期（每天/每周）自动备份到指定目录
- 备份前自动执行一轮 Agent 记忆提取，确保 KG 包含最新的会话知识

**恢复**：
- 用户选择备份包 → 解压 → 覆盖 `~/.ai-hel2/` → 重启应用 → 完整性校验（对比 manifest.json 中的 hash）

**注意**：`knowledge_cache.db` 是从 Wiki + sessions 等源文件**提取后的产物**。恢复时优先恢复源文件（Wiki + sessions），然后触发全量 re-index 重建 KG 数据库，而非直接覆盖二进制数据库文件（避免版本不兼容）。

---

## 12. Verification

- [ ] 清理后实体 2243→200-500，孤立率 <25%
- [ ] Agent 更换时 KG 模块无代码改动（Agent 只管调 nexus_store，提取逻辑全在 Nexus）
- [ ] 所有提取走 extract_service.py，Agent 不包含提取逻辑
- [ ] LLM 超时/非法JSON/API失效均不崩溃，有明确降级行为
- [ ] [[wikilink]] 实体正确继承 namespace
- [ ] EntityBrowser 按 namespace 分组折叠，未分类实体有兜底节点
- [ ] 编辑器的隐藏/删除/提升操作均写入 cache_extraction_feedback 表
- [ ] 15 条反馈后 prompt 注入偏好摘要
- [ ] 合成边灰色虚线渲染
- [ ] 本体分析建议合并相似类型
- [ ] 30 天后孤立低质实体自动删除
- [ ] 外部编辑器修改 Wiki .md 文件 → 3s 内触发提取 → 新实体入库
- [ ] 同一文件无修改保存 → content_hash 匹配 → 跳过提取（不重复入库）
- [ ] Agent 记忆手动提取：设置页触发 → sessions/ JSON → 实体/关系入库（source_type='agent_memory'）
- [ ] 备份包包含 Wiki + sessions + knowledge_cache.db + manifest.json
- [ ] 恢复：解压 → 覆盖 → re-index → 完整性校验通过
- [ ] 注册 Step 2 展示 Agent 检测结果正确（路径/版本/状态）
- [ ] 设置页"知识图谱"保存 Nexus LLM 配置到 config.yaml，重启后保持
- [ ] "测试连接"按钮返回连通状态和延迟（check_nexus_llm_connection）
- [ ] nexus_reindex_all 全量重建索引：遍历 wiki + sessions → 实体入库
