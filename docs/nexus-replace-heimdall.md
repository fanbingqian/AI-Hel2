# Nexus 完全替代 Heimdall 方案

## 架构原则

```
Agent (Python Hermes Agent, 自主决策)
  ├─ 保留工具: web_search
  └─ 知识工具(替换 heimdall_*): nexus_map · nexus_search · nexus_detail · nexus_paths · nexus_neighbors
       ↓ HTTP :18643
  Rust nexus_api.rs → knowledge_cache.db

Agent 自行判断是否调工具 → 调 nexus_map 看到完整地图 → 判断是否搜索 → 判断是否深入查询
```

---

## 第一部分：读写闭环 + Agent 工具式查询（当前实施）

> 目标：提取和查询统一到本地 SQLite，Agent 通过工具自主查询

### 1. Agent 查询方式：纯工具式

Agent 系统提示中告知 5 个 nexus 工具的可用性，不注入知识地图。Agent 完全自主决定何时调用：

```
可用工具: nexus_map · nexus_search · nexus_detail · nexus_paths · nexus_neighbors
```

Agent 决策流程（完全由 LLM 自主）：

1. 收到用户消息 → 判断是否涉及本地知识库
2. 涉及本地知识 → 调 `nexus_map` 查看知识地图（领域分布 + 关键实体 + 领域间关联）
3. 地图中有相关实体 → 调 `nexus_search` 搜索
4. 搜索结果不够 → 调 `nexus_detail` 查详情、`nexus_paths` 查关系、`nexus_neighbors` 浏览周边
5. 地图中无相关内容或知识过时 → 调 `web_search` 上网搜

**整个过程 LLM 自主决策**。地图是工具不是推送，Agent 按需拉取，不浪费上下文。`nexus_map` 返回知识地图（领域分布 + 各领域关键实体 + 子领域 + 领域间关联桥接），Agent 一次调用即可鸟瞰整个知识库的拓扑结构。

### 2. 五个 Nexus 工具

Agent 通过 HTTP 调用 Rust 端点查询（端点详情见 §10）：

| 工具 | 用途 |
|------|------|
| `nexus_map` | 查看知识地图（领域分布 + 关键实体 + 子领域 + 领域间关联桥接），Agent 据此鸟瞰知识库拓扑 |
| `nexus_search` | 全文搜索知识库实体 |
| `nexus_detail` | 查看实体完整信息 + 关系 |
| `nexus_paths` | 查找两个实体间的最短路径 |
| `nexus_neighbors` | 展开实体周边 N 跳邻居网络 |

### 3. 读路径：本地 SQLite 优先

| 函数 | 当前 | 改为 |
|------|------|------|
| `get_graph_data` | Heimdall HTTP → 本地 fallback | **本地优先 → Heimdall fallback** |
| `get_entity_detail` | Heimdall HTTP → 本地 fallback | **本地优先 → Heimdall fallback** |
| `search_entities` | Heimdall HTTP → 本地 fallback | **本地优先 → Heimdall fallback** |
| `find_entity_paths` | Heimdall HTTP → 本地 fallback | **本地优先 → Heimdall fallback** |

**Heimdall fallback 保留**：读路径翻转后，`heimdall_url` 字段及 `heimdall_url()` 访问器保留。当本地 SQLite 无结果且 Heimdall 服务仍在运行时，fallback 到 Heimdall HTTP 查询。待 Nexus 运行稳定、数据迁移验证通过后，可移除 `heimdall_url` 字段和所有 fallback 分支（作为后续清理）。

### 4. build_context_snapshot 简化 + chat.rs 移除知识注入

`build_context_snapshot` 不再注入实体详情到系统提示。其地图生成逻辑改为 `nexus_map` 工具的后端数据源：

```sql
-- 1. 领域统计 + 子领域（entity_type 分布）
SELECT namespace, COUNT(*) as cnt,
       GROUP_CONCAT(DISTINCT entity_type) as subdomains
FROM cache_entities WHERE hidden=0
GROUP BY namespace ORDER BY cnt DESC;

-- 2. 每领域 top 关键实体（按引用数+查看数排序，最多 5 个）
SELECT ce.namespace, ce.name
FROM cache_entities ce
LEFT JOIN cache_entity_scores ces ON ce.id = ces.entity_id
WHERE ce.hidden = 0
ORDER BY (COALESCE(ces.view_count, 0) + COALESCE(ces.reference_count, 0)) DESC;

-- 3. 领域间桥接（跨 namespace 的关系，按关系数量衡量强度）
SELECT ce1.namespace as domain_a, ce2.namespace as domain_b,
       COUNT(*) as relation_count
FROM cache_relations cr
JOIN cache_entities ce1 ON cr.source_id = ce1.id
JOIN cache_entities ce2 ON cr.target_id = ce2.id
WHERE ce1.namespace != ce2.namespace
  AND ce1.hidden = 0 AND ce2.hidden = 0
GROUP BY domain_a, domain_b
ORDER BY relation_count DESC;
```

`nexus_map` HTTP 端点调用这三个查询，组装为知识地图 JSON：领域分布 + 子领域 + 关键实体 + 领域间桥接。Agent 一次调用即可鸟瞰知识库拓扑。

**chat.rs 同步改动**：`chat_completions` 中删除 `build_context_snapshot` 调用（约 line 152）。Agent 的工具感知完全由 Python 端 schema 注册机制负责（与 `web_search` 一致），Rust 侧不再注入任何知识上下文或工具提醒。旧 Push 模式的知识注入与纯工具式架构矛盾，直接移除，不替换。

### 5. 写路径：全部切 Nexus

| 位置 | 当前 | 改为 |
|------|------|------|
| FileWatcher (lib.rs:214) | extract_entities() → Heimdall HTTP | nexus_extract_from_file() |
| 启动扫描 (k_s.rs:117) | scan_wiki_directory → 正则 | 内部改为 nexus_extract_from_file() |
| nexus 失败 fallback (k_s.rs:2162) | extract_entities_local 正则 | 保留兜底，加 log 标记 |

**FileWatcher 旧保护逻辑移除**（`file_watcher.rs`）：

| 保护 | 位置 | 原因 | 处理 |
|------|------|------|------|
| Protection 1: 跳过 `heimdall/` 子目录 | `file_watcher.rs:63` | 旧管道中 Heimdall 目录下文件由 Heimdall 管理，跳过避免重复提取 | 删除 — Nexus 不需要此保护 |
| Protection 2: 跳过 `heimdall_id:` + `auto-generated` 标记 | `file_watcher.rs:67-77` | 旧管道中 save_chat_to_knowledge 生成的 .md 带这些 YAML frontmatter 标记，跳过避免 FileWatcher 再次提取 | 删除 — 新管道不再写入这些标记 |

**save_chat_to_knowledge 同步变化**：停止在生成的 .md 文件中写入 `heimdall_id` 和 `auto-generated` YAML frontmatter 标记。这些标记是旧管道 FileWatcher 保护逻辑的配套措施，Nexus 管道不再需要。

### 6. 旧命令转调

| 命令 | 改为 |
|------|------|
| extract_entities | 内部转调 nexus_extract_from_file |
| extract_entities_from_text | 内部转调 nexus_store |
| rescan_wiki | scan_wiki_directory 已改 |

### 7. 聊天知识提取：文件保存后自动触发

聊天过程不直接触发提取。聊天相关的知识以文件形式进入 wiki 目录后，由 FileWatcher 自动触发 Nexus 提取：

**两个来源**：

| 来源 | 文件名 | 触发方式 |
|------|--------|---------|
| Agent 生成的文件 | Agent 对话中生成的 .md（代码/报告等），Agent 自行命名 | 保存到 `wiki/` → FileWatcher 检测 → Nexus 提取 |
| Agent 记忆文件 | Hermes Agent 的 session 文件（`session_api-{id}.json`） | 用户手动"同步对话" → 转为 .md 存入 `wiki/chat/` → FileWatcher 检测 → Nexus 提取 |

**chat:done 改动**：移除 `nexus_store` + `save_chat_to_knowledge` 调用，保留 `add_message`（保存消息到 SQLite）和 `generate_title`（首次回复后异步生成 session 标题，标题用于手动同步对话时的 .md 文件命名）。

**手动同步**：知识编辑页新增"同步对话"按钮，列出可同步的 session，用户选择后：
1. 读取 session 的完整 messages
2. 拼接为对话文本
3. 生成 .md 文件写入 `wiki/chat/`
4. FileWatcher 自动检测新文件 → 触发 `nexus_extract_from_file`

**文件命名**：聊天同步的 .md 文件名直接用 session 标题（已有），无需 AI 重新生成标题。标题为空时用用户第一条消息的前 30 字。

### 8. 文件命名统一

- 聊天文件: `{session_title}_{date}.md`，存入 `wiki/chat/`
- YAML title = session 标题（非固定"对话记录"）
- save_chat_to_knowledge 简化为纯文件生成

### 9. 命名空间类型化

- Entity / FileNode 接口增加 namespace 字段
- 前端接入 get_namespaces 命令
- EntityBrowser 移除 (e as any).namespace
- DocTree 显示文件名为主，namespace 为副标题

### 10. Agent 工具实现方式

Rust 端开一个本地 HTTP 端点（端口 18643），Agent 通过 HTTP 调 Rust 查询 `knowledge_cache.db`：

```
Agent (Python)
  → nexus_search("Nginx")
  → HTTP GET http://127.0.0.1:18643/nexus/search?q=Nginx
  → Rust handle → knowledge_cache.db → FTS5
  → 返回 JSON {entities: [...]}
```

**五个端点**：

| 工具 | HTTP 端点 | 后端函数 |
|------|----------|---------|
| `nexus_map` | `GET /nexus/map` | 查询领域分布 + 子领域 + 关键实体 + 领域间桥接 |
| `nexus_search` | `GET /nexus/search?q=&namespace=&limit=` | `search_entities_local`（FTS5） |
| `nexus_detail` | `GET /nexus/entity/{id}` | `get_entity_detail_local` |
| `nexus_paths` | `GET /nexus/paths?from=&to=&max_hops=` | 接受实体名称或 UUID，名称先经 FTS5 模糊匹配→ID，再 BFS |
| `nexus_neighbors` | `GET /nexus/neighbors/{id}?hops=` | 读 entity + relations N 跳 BFS |

**注意**：`nexus_paths` 的 `from`/`to` 参数同时接受实体名称（模糊匹配）和 UUID 精确匹配。后端逻辑：先判断是否为 UUID 格式 → 是则直接用 → 否则 FTS5 模糊匹配名称→ID → BFS。

**nexus_map 响应格式（知识地图）**：

```json
{
  "knowledge_map": {
    "total_entities": 135,
    "total_relations": 200,
    "domains": [
      {
        "name": "技术/开发工具",
        "entity_count": 89,
        "key_entities": ["Nginx", "Docker", "Python", "Kubernetes", "Redis"],
        "subdomains": ["容器技术", "Web服务器", "数据库", "CI/CD", "编程语言"],
        "connected_to": ["业务/金融"]
      },
      {
        "name": "业务/金融",
        "entity_count": 34,
        "key_entities": ["营收模型", "Q2财报", "客户画像", "市场分析", "ROI计算"],
        "subdomains": ["财务分析", "市场研究", "客户管理"],
        "connected_to": ["技术/开发工具"]
      },
      {
        "name": "科学/AI",
        "entity_count": 12,
        "key_entities": ["LLM", "RAG", "Embedding", "Transformer", "Fine-tuning"],
        "subdomains": ["NLP", "深度学习"],
        "connected_to": ["技术/开发工具"]
      }
    ],
    "bridges": [
      {"domain_a": "技术/开发工具", "domain_b": "业务/金融", "strength": "中", "relation_count": 12, "example": "数据分析 → 营收模型"},
      {"domain_a": "技术/开发工具", "domain_b": "科学/AI", "strength": "强", "relation_count": 25, "example": "Python → LLM"}
    ]
  }
}
```

- `key_entities`：每域名前 5 个高频实体（按 `view_count + reference_count` 排序）
- `subdomains`：域内 `entity_type` 去重聚合
- `connected_to`：有跨域关系的目标域
- `bridges`：跨域关系详情，`strength` 按 `relation_count` 分档：强(≥20) / 中(≥5) / 弱(<5)

**Rust 端**：`src-tauri/src/services/nexus_api.rs`（新建），在 Tauri setup 阶段 spawn 独立线程运行 `tiny_http` server，只监听 `127.0.0.1:18643`。线程生命周期由 Tauri `App::on_exit` 管理。端口启动前做 bind 检测，占用时尝试递增端口。

**Cargo 依赖**：在 `src-tauri/Cargo.toml` 中添加 `tiny_http = "0.12"`。

**启动顺序约束**：`KnowledgeService` Arc 必须在 `nexus_api::start()` 调用前就绪。Tauri setup 中先初始化 KnowledgeService → 传入 Arc 再启动 HTTP server。

**端口通信机制**：Rust 启动成功后把实际端口写入 `{hermes_home}/nexus_port` 文件（纯文本，如 `18643`）。Python 端 `nexus_tools.py` 导入时读取该文件获取端口。文件不存在时（Nexus HTTP server 未启动），5 个 nexus 工具不注册，Agent 回退到只有 `web_search`。

**Python 端**：`heimdall/tools/nexus_tools.py`（新建），5 个函数各调一个 HTTP 端点，在 `tools/registry.py` 注册。**同时移除旧 `heimdall_knowledge`、`heimdall_persona`、`heimdall_memory` 三个工具的注册并删除这三个文件**（`knowledge_tool.py`、`persona_tool.py`、`memory_tool.py`），Agent 只保留 `web_search` + 5 个 nexus 工具。

**provider.py 同步改动**：
- `HEIMDALL_GUIDANCE`（lines 63-75）：替换为 `NEXUS_GUIDANCE`，简述 5 个 nexus 工具的用途和自主调用原则
- `system_prompt_block()`（lines 246-281）：移除旧 Heimdall 系统提示块，改为轻量提示——告知 Agent 有 5 个 nexus 知识工具可用，`nexus_map` 可查看完整知识地图，其余工具按需调用，Agent 完全自主决策

**run_agent.py 同步改动**：
- 删除 `HeimdallManager` 注册代码块（`run_agent.py:2082-2122`，含 `[HEIMDALL CUSTOM]` 注释段），约 40 行。Agent 启动时不再初始化任何 Heimdall 相关 MemoryProvider。旧 heimdall_memory/heimdall_persona/heimdall_knowledge 工具的后端已由 nexus 工具替代，注册代码无存在意义。git 历史可恢复，无需注释保留。

**优势**：Rust 是 `knowledge_cache.db` 的唯一读写者，schema 变更不影响 Python。与现有 Heimdall HTTP 架构一致。curl 可独立测试。

**工具 JSON Schema（OpenAI function calling 格式）**：

```json
// nexus_map — 查看知识地图
{
  "type": "function",
  "function": {
    "name": "nexus_map",
    "description": "返回本地知识库的知识地图：领域分布、各领域关键实体、子领域结构、领域间关联桥接。Agent 据此鸟瞰知识库拓扑，判断覆盖范围，决定是否需要 nexus_search 深入搜索。",
    "parameters": {
      "type": "object",
      "properties": {},
      "required": []
    }
  }
}

// nexus_search — 全文搜索知识库实体
{
  "type": "function",
  "function": {
    "name": "nexus_search",
    "description": "全文搜索本地知识库中的实体。返回匹配实体列表，含摘要信息。在 nexus_map 确认知识库有相关覆盖后使用，也可直接调用（跳过 nexus_map）。Agent 根据结果判断是否需要 nexus_detail 深入查看。",
    "parameters": {
      "type": "object",
      "properties": {
        "q": {"type": "string", "description": "搜索关键词"},
        "namespace": {"type": "string", "description": "可选，限定命名空间过滤"},
        "limit": {"type": "integer", "description": "返回数量上限，默认 10"}
      },
      "required": ["q"]
    }
  }
}

// nexus_detail — 查看实体完整信息
{
  "type": "function",
  "function": {
    "name": "nexus_detail",
    "description": "获取单个实体的完整信息，包括属性、入边关系（哪些实体指向它）、出边关系（它指向哪些实体）。用于 nexus_search 结果中感兴趣实体的深入查看。",
    "parameters": {
      "type": "object",
      "properties": {
        "id": {"type": "string", "description": "实体 ID（nexus_search 返回结果中的 entity id）"}
      },
      "required": ["id"]
    }
  }
}

// nexus_paths — 查找实体间最短路径
{
  "type": "function",
  "function": {
    "name": "nexus_paths",
    "description": "查找两个实体之间的最短关系路径（BFS，最多 4 跳）。用于理解概念之间的关联链条。from/to 参数同时接受实体名称（模糊匹配）和 UUID（精确匹配）。",
    "parameters": {
      "type": "object",
      "properties": {
        "from": {"type": "string", "description": "起始实体名称或 UUID（名称做模糊匹配，UUID 精确匹配）"},
        "to": {"type": "string", "description": "目标实体名称或 UUID（名称做模糊匹配，UUID 精确匹配）"},
        "max_hops": {"type": "integer", "description": "最大跳数，默认 4"}
      },
      "required": ["from", "to"]
    }
  }
}

// nexus_neighbors — 展开周边邻居网络
{
  "type": "function",
  "function": {
    "name": "nexus_neighbors",
    "description": "展开指定实体周边 N 跳的邻居网络（BFS）。用于浏览知识图谱中某个实体周围的相关概念。",
    "parameters": {
      "type": "object",
      "properties": {
        "id": {"type": "string", "description": "实体 ID"},
        "hops": {"type": "integer", "description": "展开跳数，默认 2，最大 4"}
      },
      "required": ["id"]
    }
  }
}
```

### 11. 数据迁移：heimdall.db → knowledge_cache.db

读路径翻转为本地优先后，`knowledge_cache.db` 必须有数据。需要一次性迁移 `heimdall.db` 中的历史知识。

**迁移策略：首次启动自动执行**

1. 检查 `knowledge_cache.db` 中 `cache_entities` 表是否为空（或 `cache_schema_version = 0`）
2. 为空 → 检查 `{hermes_home}/heimdall/heimdall.db` 是否存在
3. 存在 → 读取 `heimdall_entities` 表，映射到 `cache_entities` 表结构
4. 同时迁移 `heimdall_memory_edges` → `cache_relations`
5. 迁移完成后写入 `cache_schema_version`，后续启动跳过

**表映射**：

| heimdall.db 表 | knowledge_cache.db 表 | 字段映射 |
|----------------|----------------------|---------|
| `heimdall_entities` | `cache_entities` | id→id, name→name, description→description, entity_type→entity_type, namespace→namespace, confidence→confidence, created_at→created_at, updated_at→updated_at |
| `heimdall_memory_edges` | `cache_relations` | source_id→source_id, target_id→target_id, relation_type→relation_type, weight→weight |
| 无对应（新建） | `cache_entity_scores` | entity_id=实体ID, view_count=0, reference_count=迁移关系计数 |

**迁移日志**：迁移过程写入 `{hermes_home}/logs/nexus_migration.log`，记录迁移实体数、关系数、耗时。

**Heimdall.db 保留**：迁移后 `heimdall.db` 不删除，重命名为 `heimdall.db.migrated.{date}` 作为备份。用户确认正常后可手动删除。

**失败处理**：迁移失败时 Agent 仍可通过 Heimdall HTTP fallback 读取旧数据（若 Heimdall 仍在运行），不影响使用。

### 12. wiki 路径切换：`heimdall/wiki/` → `wiki/`

Nexus 完全替代 Heimdall 后，wiki 文件不应再存放在 `heimdall/` 子目录下。

**路径变更**：

| 位置 | 当前路径 | 改为 |
|------|---------|------|
| `lib.rs:44` | `hermes_home.join("heimdall").join("wiki")` | `hermes_home.join("wiki")` |
| `lib.rs:57` | `WikiService::new(&hermes_home.join("heimdall").join("wiki"))` | `WikiService::new(&hermes_home.join("wiki"))` |
| `commands/config.rs:103` | `home.join("heimdall").join("wiki")` | `home.join("wiki")` |
| `knowledge_service.rs:74` | `hermes_home.join("heimdall").join("wiki")` | `hermes_home.join("wiki")` |
| Python `plugins/memory/aihel/` | 引用 `heimdall/wiki/` | 引用 `wiki/` |

**旧文件迁移**：启动时检查 `heimdall/wiki/` 是否存在 → 将其中所有 .md 文件移动到 `wiki/` 根目录 → 迁移完成后删除空的 `heimdall/wiki/` 目录。保留 `heimdall/` 目录本身（`heimdall.db.migrated.{date}` 备份文件在其中）。

### 13. CSP 清理

`tauri.conf.json` 的 CSP `connect-src` 中移除 `http://127.0.0.1:8765`（旧 Heimdall 端口）。

Nexus HTTP server（`127.0.0.1:18643`）不加入 CSP。前端通过 Tauri `invoke()` IPC 与 Rust 通信，不直接跨域调用 Nexus HTTP。Nexus HTTP 的调用者是 Python Agent，不受浏览器 CSP 限制。

### 实施步骤

| Step | 内容 | 涉及文件 |
|------|------|---------|
| **S1** | heimdall.db → knowledge_cache.db 自动迁移（先迁移数据再翻转读路径） | `knowledge_service.rs` |
| **S2** | 读路径 4 函数翻转本地优先 | `knowledge_service.rs` |
| **S3** | build_context_snapshot 简化 + nexus_map 数据源 + chat.rs 移除知识注入 | `knowledge_service.rs`, `chat.rs` |
| **S4** | FileWatcher 切换 Nexus + 移除旧保护逻辑 | `lib.rs`, `file_watcher.rs` |
| **S5** | scan_wiki_directory 切换 Nexus | `knowledge_service.rs` |
| **S6** | 旧命令内部转调 | `commands/knowledge.rs` |
| **S7** | save_chat_to_knowledge 简化 + 命名规则 + 移除旧标记 | `knowledge_service.rs` |
| **S8** | chatStore 移除直接提取调用 | `chatStore.ts` |
| **S9** | TypeScript 类型补充 | `types/knowledge.ts`, `types/wiki.ts` |
| **S10** | 前端接入 get_namespaces | `api.ts`, `EntityBrowser.tsx` |
| **S11** | DocTree 显示逻辑更新 | `DocTree.tsx` |
| **S12** | 知识编辑页"同步对话"按钮 | `KnowledgeEditor.tsx` |
| **S13** | Rust HTTP 端点 nexus_api.rs 新建（含启动顺序+端口文件+Cargo依赖） | `nexus_api.rs`(新建), `lib.rs`, `Cargo.toml` |
| **S14** | Python 端新增 nexus_tools.py + 注册（替换 heimdall_* 并删除旧文件 + run_agent.py 移除 HeimdallManager） | `nexus_tools.py`(新建), `registry.py`, `provider.py`, `run_agent.py` |
| **S15** | wiki 路径切换：`heimdall/wiki/` → `wiki/` | `lib.rs`, `knowledge_service.rs`, `commands/config.rs` |
| **S16** | CSP 清理：移除 `http://127.0.0.1:8765` | `tauri.conf.json` |

---

## 第二部分：Nexus 高级能力（后续 N1-N5）

> 目标：Nexus 自主运行知识全生命周期

### Nexus 全能力对照

| 能力 | 实现方式 | 类型 |
|------|---------|------|
| 实体提取 | extract_service.py ✅ 已有 | LLM |
| 图谱查询 | 本地 SQLite ✅ 已有 | Rust |
| 合成推理 (3规则) | nexus_run_synthesis ✅ 已有 | Rust |
| 类型分析 | nexus_analyze_types ✅ 已有 | Rust |
| PageRank | N1 新增 — 纯图算法 | Rust |
| 置信度衰减/增强 | N1 新增 — 数学公式 | Rust |
| 过期扫描/清理 | N1 新增 — SQL 查询 | Rust |
| 传递性推理 | N1 新增 — 规则引擎 | Rust |
| 社区检测 | N1 新增 — Louvain 算法 | Rust |
| 推理验证 | N2 新增 — infer_service.py | LLM |
| 实体去重合并 | N2 新增 — dedup_service.py | LLM |
| 冲突检测 | N3 新增 — conflict_service.py | LLM |
| 观点演化总结 | N3 新增 — evolve_service.py | LLM |
| 因果链 | N1 新增 — BFS 路径 | Rust |

### 调度

| 任务 | 频率 | 触发 |
|------|------|------|
| 置信度衰减 | 每日 | 定时 |
| PageRank 重算 | 每周 | 定时 |
| 过期扫描 | 每日 | 定时 |
| 传递性推理 | 新关系入库 | 事件 |
| 实体去重检查 | 新实体入库 | 事件 |
| 冲突检测 | 实体更新 | 事件 |
| 演化总结 | 手动 | 手动 |
| 社区检测 | 每月 | 定时 |

---

## 验证

### 第一部分验证
- [ ] Agent 可通过 nexus_map 查看知识地图（领域分布+关键实体+子领域+桥接）
- [ ] Agent 可通过 nexus_search/nexus_detail/nexus_paths/nexus_neighbors 查询本地知识
- [ ] 旧 heimdall_knowledge/heimdall_persona/heimdall_memory 工具文件已删除，注册已移除
- [ ] provider.py 中 HEIMDALL_GUIDANCE 已替换为 NEXUS_GUIDANCE
- [ ] provider.py 中 system_prompt_block() 已替换为 nexus 轻量提示
- [ ] run_agent.py 中 HeimdallManager 注册代码块已删除
- [ ] chat.rs 中 build_context_snapshot 调用已移除，无替换
- [ ] Agent 的 web_search 工具仍正常工作，不被本地工具挤掉
- [ ] FileWatcher 触发 Nexus extract_service.py 提取
- [ ] 启动扫描走 Nexus
- [ ] heimdall.db → knowledge_cache.db 首次迁移正常执行，迁移日志记录完整
- [ ] 迁移后 heimdall.db 重命名为 .migrated.{date} 备份
- [ ] 聊天同步的 .md 文件以 session 标题命名，存入 wiki/chat/
- [ ] 标题为空的 session 使用用户第一条消息前 30 字作为文件名
- [ ] DocTree 显示正确文件名
- [ ] EntityBrowser 无 (e as any).namespace
- [ ] Nexus HTTP server 启动时端口检测正常，失败时有降级日志
- [ ] `{hermes_home}/nexus_port` 文件写入正确端口，Python 端可读取
- [ ] KnowledgeService Arc 在 nexus_api 启动前就绪，无竞态条件
- [ ] FileWatcher 不再跳过 heimdall/ 子目录文件
- [ ] FileWatcher 不再检查 heimdall_id / auto-generated 标记
- [ ] save_chat_to_knowledge 生成的 .md 不再包含 heimdall_id / auto-generated
- [ ] nexus_port 文件不存在时 5 个 nexus 工具不注册，Agent 仅 web_search
- [ ] nexus_paths 同时接受实体名称和 UUID 两种参数
- [ ] nexus_map 返回正确的 JSON 结构（domains+bridges）
- [ ] wiki 路径已从 `heimdall/wiki/` 切换到 `wiki/`
- [ ] 旧 `heimdall/wiki/` 中 .md 文件已自动迁移到新路径
- [ ] tauri.conf.json CSP 中 `http://127.0.0.1:8765` 已移除
- [ ] Heimdall 未运行时所有核心操作正常

### 第二部分验证
- [ ] PageRank 分数反映实体真实重要性
- [ ] 低价值实体随时间自动衰减
- [ ] 传递性推理产生合理候选
- [ ] LLM 去重正确识别重复实体
- [ ] 定时任务不阻塞 UI
