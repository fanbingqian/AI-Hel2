# Nexus 自动知识库维护与推理方案

## 角色定位

Nexus 扮演双重角色：

- **图书管理员** — 负责入库、归档、去重、修补。Agent 只管借书看书。
- **研究员** — 从已有知识中推导新知识、检测模式、发现冲突、追踪观点演化。

任何 Agent 接入时拿到的都是 Nexus 维护好的干净知识库 + 推理好的衍生知识。

---

## 1. 维护任务清单

| 任务 | 做什么 | LLM 介入程度 |
|------|--------|-------------|
| **去重合并** | 检测相似实体 → LLM 判断是否同一实体 → 合并 | 需要 LLM 判断 |
| **质量评分** | 缺描述、缺关系、描述过短的实体标记低质量 | 不需要 LLM，规则即可 |
| **孤岛清理** | 0 入边 0 出边的孤立实体 → 归档 | 不需要 LLM |
| **分类纠错** | namespace 明显不对的实体 → 修正；`"concept"` 带引号类型 → 去引号 | 轻量 LLM（分类任务） |
| **过期检测** | 源文件已删 + 长期未访问 → 标记 stale | 不需要 LLM |
| **迁移修复** | `heimdall_migrated` 来源且 `llm_confidence=0` → 标记待重提取 | 不需要 LLM，规则即可 |

---

## 2. 执行时机与策略

### 2.1 触发时机

```
时机 A: 文件上传后（增量）
  → 只处理本次新提取的实体（通常 < 20 个）
  → 去重检查：新实体 vs 已有同名/相似实体
  → 预计 token: < 2000（~10 次 LLM 判断 × 200 tokens）

时机 B: 每日凌晨（轻量扫描）
  → 合成推理（3 条规则）→ 质量评分 + 孤岛检测 + 过期检测 + PageRank 重算
  → 全规则 + 图算法，不用 LLM（合成推理走 SQL）
  → 预计 token: 0
  → 合成出新的低置信度边后，自动触发推理验证

时机 C: 手动触发（全量深度整理）
  → 用户在设置页点"知识库整理"
  → 去重合并 + 分类纠错 + 质量评分
  → 预计 token: 取决于数据量，需控制
```

### 2.2 增量 vs 全量

| | 增量（自动） | 全量（手动） |
|------|------------|------------|
| 触发 | 文件上传后 | 用户手动 |
| 范围 | 新实体 + 相关邻居 | 全库 |
| LLM 用量 | 极低 | 需控制 |
| 去重 | ✅ 新 vs 已有 | ✅ 全库 pairwise |

**原则：自动只做增量，全量必须手动触发。**

### 2.3 首次全量整理（特殊场景）

当知识库存在大量遗留数据（如 Heimdall 迁移、`llm_confidence=0` 占比 > 50%）时，需要一次**首次全量整理**：

```
1. 迁移修复: 扫描所有 heimdall_migrated + confidence=0 → 标记待重提取
2. 引号类型清理: "concept" → concept, "person" → person
3. 全量去重: 用漏斗过滤 + LLM 判断
4. 质量评分: 全部实体一次扫描
5. 孤岛归档: 检测到的孤岛 → hidden=1（首次整理无 PageRank，全量归档；后续维护中结合 PageRank 保护核心实体）
6. 重提取: 对有 source_path 且 confidence=0 的实体重新调用 extract_service.py
```

**执行时机**: 用户在设置页点"首次整理"按钮，或维护系统首次运行时检测到低质量占比 > 50% 时自动提示。

**预计 Token**: 去重 LLM 调用 ~10 次（约 2000 tokens），重提取按实体数计算。

---

## 3. Token 控制策略

### 3.1 核心问题

全量去重是 O(n²) — N 个实体两两比较 = N×(N-1)/2 对，不可能全送 LLM。

### 3.2 漏斗过滤（去重任务）

```
全量实体 (N)
  │
  ├─ 过滤 1: 同名/近名（name 相似度 > 0.8）        → 候选 ~N×0.02 对
  ├─ 过滤 2: 同 namespace + 同 entity_type          → 候选 ~N×0.01 对
  ├─ 过滤 3: FTS5 搜索匹配                          → 候选 ~N×0.008 对
  │
  └─ 送入 LLM 判断（每对 ~200 tokens）              → ~N×1.6 tokens（N=2500 时约 4000）
```

LLM 只做最终判断，前面全是 SQL/规则过滤。

### 3.3 分批 + 上限

```
每个维护周期:
  ├─ 最大处理批次: 50 个实体/次
  ├─ 最大 LLM 调用: 10 次/次
  ├─ 最大 token 预算: 20000 tokens/次
  └─ 未完成的标记 pending，下次继续
```

### 3.4 LLM 只做"判断"，不做"生成"

去重 prompt 设计为选择题，控制输出长度：

```
以下是两个知识库实体，判断它们是否同一个概念，只回复 YES 或 NO：

实体 A: [name: "深度学习", type: "concept", desc: "机器学习子领域..."]
实体 B: [name: "Deep Learning", type: "concept", desc: "利用多层神经网络..."]

→ "YES"
```

每次调用 ~200 input tokens + 1 output token。

### 3.5 数据增长后的策略

| 实体数量 | 策略 |
|---------|------|
| < 1000 | 全量质量扫描 OK |
| 1000-5000 | namespace 分区扫描，每区独立 |
| 5000-20000 | 仅扫 `updated_at > 上次维护时间` 的实体 |
| > 20000 | 随机采样 10% + 优先扫高分实体 |

---

## 4. 推理与智能能力

维护是"守"，推理是"攻"。Nexus 不仅要保持知识库干净，还要主动发现新知识、检测模式、追踪观点演化。

### 4.1 合成推理（已实现）

当前代码中 `nexus_run_synthesis()` 已实现 3 条规则，位于 [knowledge_service.rs:3189](src-tauri/src/services/knowledge_service.rs#L3189)。

**规则 1: 共享邻居推断**

```
如果 A → X ← B，且 X 的度数 ≥ 3（枢纽节点），A-B 之间无直接边
→ 推断 A —[related_to]→ B，置信度 0.25
```

这是"朋友的朋友可能是朋友"的图结构推理。只对高度数枢纽节点触发，避免低质量边。

**规则 2: 共现推断**

```
如果 A 和 B 在 ≥ 3 个不同的 source_path 中共现
→ 推断 A —[co_occurs]→ B，置信度 0.2 + k×0.05（k = 共现 source_path 数）
```

同一个文档/对话里反复一起出现的实体，即使没有显式关系也推断有关联。

**规则 3: 类型模式发现**

```
如果某 entity_type 的实体 ≥ 80% 共享同一 relation_type
→ 记录模式到 cache_ontology（如 "person" 类型 90% 有 "works_at" 关系）
```

这是本体论自动归纳 — 从数据中学习类型约束。

**LLM 介入程度**: 0。三条规则全是 SQL + 图遍历，不需要 LLM。

---

### 4.2 图智能（计划中）

来自 [nexus-replace-heimdall.md](docs/nexus-replace-heimdall.md) N1 的设计。

#### PageRank 实体重要性

```
纯图算法，不需要 LLM

1. 所有实体初始 PR = 1/N
2. 迭代: PR(A) = (1-d)/N + d × Σ(PR(B) / out_degree(B))  for B → A
3. 收敛后: PR > 均值×2 → "核心实体" 标签
4. 写入 cache_entity_scores.importance_score
```

**用途**: 知识地图中高亮关键实体、搜索排序加权、孤岛清理时保护重要实体。

**执行时机**: 每日凌晨轻量扫描（时机 B），全量重算。

#### Louvain 社区检测

```
纯图算法，不需要 LLM

1. 第一阶段: 每个节点依次尝试移入邻居社区，选模块度增益最大的
2. 第二阶段: 将社区收缩为超节点
3. 重复直到模块度不再提升
4. 写入 cache_entities.community_id
```

**用途**: 知识地图中自动发现"领域"（可能比人工 namespace 更准确），检测跨域桥梁实体。

**执行时机**: 全量手动触发（时机 C），O(n log n) 不需要 LLM。

#### 因果链发现

```
纯 BFS 路径搜索，不需要 LLM

从一个事件实体出发:
  ├─ 沿 cause→effect 边正向 BFS → 发现"后果链"
  ├─ 沿 cause→effect 边反向 BFS → 发现"根因链"
  └─ 返回 N 跳因果路径
```

**用途**: 回答"X 导致了什么"和"什么导致了 X"。

**执行时机**: Agent 调用 `/nexus/paths` 时按需触发（relation_type 过滤为 cause/leads_to 等因果边）。

---

### 4.3 知识演进（计划中）

来自 [nexus-replace-heimdall.md](docs/nexus-replace-heimdall.md) N1-N3 的设计。

#### 传递推理

```
规则引擎，不需要 LLM

if A —[is_a]→ B and B —[is_a]→ C:
    → A —[is_a]→ C, 置信度 = min(conf_AB, conf_BC) × 0.9

if A —[part_of]→ B and B —[part_of]→ C:
    → A —[part_of]→ C, 置信度 = min(conf_AB, conf_BC) × 0.85
```

只对传递性关系类型（`is_a`, `part_of`, `located_in`, `belongs_to` 等）执行。

**LLM 介入程度**: 0，纯规则传递闭包。

#### 冲突检测

```
漏斗过滤 → LLM 判断

1. SQL 筛选: 两个实体之间存在多条 relation_type 互斥的边
   互斥对: supports/opposes, agrees/disagrees, proves/disproves
2. 送入 LLM: "以下两个实体之间存在矛盾信息，判断是否真的冲突"
3. 如冲突 → 标记到 cache_ontology，降低两者置信度
```

**Token 估算**: 互斥边对极少（< 1% 的边），每次全量扫描候选 < 10 对，token 极低。

**执行时机**: 全量手动触发（时机 C）。

#### 观点演化追踪

```
LLM 总结，需要 LLM

1. SQL 筛选: 同 namespace + 同 entity_type + created_at 跨度 > 7 天
2. 按时间窗口分组（周/月）
3. 送入 LLM: "以下是'{实体名}'在不同时间段的描述，总结观点变化"
4. 生成演化时间线 → 写入 cache_synthesis
```

**前置依赖**: 需要 `cache_entity_snapshots` 表存储每次提取/更新时的 description 快照（entity_id + desc + captured_at）。当前 `cache_entities.desc` 只有一个版本，需要增加快照机制。

**Token 估算**: 每次只追一个实体的演化，~500 input + ~200 output tokens。

**执行时机**: 手动触发，针对特定实体（用户右键菜单"查看演化"）。

---

### 4.4 推理验证（计划中）

来自 [nexus-replace-heimdall.md](nexus-replace-heimdall.md) N2 的设计。

合成推理产生的边置信度低（0.2-0.35），需要 LLM 二次验证才能提升到高置信度。

```
合成边（低置信度）
  │
  ├─ 过滤: confidence < 0.4 且 source_type = "synthesis"
  ├─ 批量: 每批 10 条
  └─ 送入 LLM:
      "以下是 10 条从知识图谱推导出的关系，逐条判断是否成立:
       1. [实体A] —[related_to]→ [实体B]  依据: 共享邻居 [X]
       2. ...
       回复格式: 1. YES/NO  2. YES/NO  ..."
```

**Token 控制**: 每批 10 条 ~800 input + ~20 output tokens。合成边通常 < 实体数的 10%（密集图上限，稀疏图实际约 3-5%），N 个实体约 N×0.05 条合成边待验证。

**验证通过 → 置信度提升到 0.7 + source_type 改为 verified。**

**执行时机**: 合成推理后自动触发（时机 B 的一部分）。

---

### 4.5 能力总览

```
Nexus 智能层级:

Layer 0: 数据入库 ─── extract_service.py, nexus_store()     [已实现]
Layer 1: 基础查询 ─── search, paths, neighbors, map         [已实现]
Layer 2: 质量维护 ─── 去重, 质量评分, 孤岛, 分类, 过期, 迁移修复 [计划中]
Layer 3: 合成推理 ─── 共享邻居, 共现, 类型模式               [已实现]
Layer 4: 图智能   ─── PageRank, 社区检测, 因果链            [计划中]
Layer 5: 知识演进 ─── 传递推理, 冲突检测, 观点演化           [计划中]
Layer 6: 推理验证 ─── LLM 二次验证合成边                    [计划中]
```

---

## 5. 实现

### 5.1 新增文件（维护 + 推理）

```
src-tauri/src/services/
├─ maintain_dedup.py          # LLM 去重判断脚本
├─ maintain_quality.py        # 质量评分（纯规则）
├─ maintain_cleanup.py        # 孤岛+过期+迁移修复（纯规则）
├─ infer_service.py           # LLM 推理验证脚本（冲突检测 + 观点总结 + 合成边验证）
├─ nexus_maintenance.rs       # 编排维护流程，spawn Python 脚本
├─ nexus_synthesis.rs         # 合成推理引擎（从 knowledge_service.rs 提取，纯 SQL）
├─ nexus_graph_intel.rs       # PageRank + Louvain 社区检测 + 因果链
├─ nexus_evolution.rs         # 传递推理 + 冲突检测 + 观点演化
└─ nexus_verify.rs            # LLM 推理验证编排（spawn infer_service.py）
```

### 5.2 新增 Nexus API 端点

```
# 维护
GET  /nexus/maintain/health    → 检查 Nexus HTTP 服务是否在线
POST /nexus/maintain/dedup     → 手动触发去重（使用 LLM）
POST /nexus/maintain/quality   → 质量评分（无 LLM）
POST /nexus/maintain/cleanup   → 孤岛+过期清理（无 LLM）
POST /nexus/maintain/fix-migrated → 修复 heimdall_migrated 置信度为 0 的实体
GET  /nexus/maintain/status    → 上次维护时间 + 结果摘要

# 推理
POST /nexus/synthesis/run      → 运行合成推理（3 条规则）
POST /nexus/synthesis/verify   → LLM 验证合成边
GET  /nexus/synthesis/status   → 合成边数量 + 待验证数
POST /nexus/pagerank           → 计算 PageRank（手动触发）
POST /nexus/community          → Louvain 社区检测（手动触发）
GET  /nexus/evolution/{id}     → 查看实体观点演化时间线
POST /nexus/conflict/scan      → 冲突检测扫描（手动触发）
```

### 5.3 新增 Tauri 命令

```rust
// 维护
#[tauri::command]
async fn nexus_maintain_dedup(state: State<'_, KnowledgeState>) -> Result<MaintenanceReport, String>

#[tauri::command]
async fn nexus_maintain_quality(state: State<'_, KnowledgeState>) -> Result<QualityReport, String>

#[tauri::command]
async fn nexus_maintain_cleanup(state: State<'_, KnowledgeState>) -> Result<CleanupReport, String>

#[tauri::command]
async fn nexus_maintain_fix_migrated(state: State<'_, KnowledgeState>) -> Result<FixMigratedReport, String>

#[tauri::command]  
async fn nexus_get_maintenance_status(state: State<'_, KnowledgeState>) -> Result<MaintenanceStatus, String>

// 推理
#[tauri::command]
async fn nexus_run_synthesis(state: State<'_, KnowledgeState>) -> Result<SynthesisReport, String>

#[tauri::command]
async fn nexus_verify_synthesis(state: State<'_, KnowledgeState>) -> Result<VerifyReport, String>

#[tauri::command]
async fn nexus_run_pagerank(state: State<'_, KnowledgeState>) -> Result<PageRankReport, String>

#[tauri::command]
async fn nexus_run_community(state: State<'_, KnowledgeState>) -> Result<CommunityReport, String>

#[tauri::command]
async fn nexus_get_evolution(entity_id: String, state: State<'_, KnowledgeState>) -> Result<EvolutionTimeline, String>

#[tauri::command]
async fn nexus_scan_conflicts(state: State<'_, KnowledgeState>) -> Result<ConflictReport, String>
```

### 5.4 维护与推理记录表

```sql
-- 维护日志
CREATE TABLE IF NOT EXISTS cache_maintenance_log (
    id TEXT PRIMARY KEY,
    task TEXT NOT NULL,          -- dedup / quality / cleanup / classify / stale / fix_migrated
    started_at TEXT NOT NULL,
    completed_at TEXT,
    entities_scanned INTEGER,
    entities_fixed INTEGER,
    llm_calls INTEGER,
    tokens_used INTEGER,
    status TEXT DEFAULT 'running' -- running / completed / failed
);

-- 实体描述快照（观点演化追踪依赖）
CREATE TABLE IF NOT EXISTS cache_entity_snapshots (
    id TEXT PRIMARY KEY,
    entity_id TEXT NOT NULL,
    desc TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    source_path TEXT,
    FOREIGN KEY (entity_id) REFERENCES cache_entities(id)
);

-- 推理日志
CREATE TABLE IF NOT EXISTS cache_synthesis_log (
    id TEXT PRIMARY KEY,
    task TEXT NOT NULL,          -- synthesis / verify / conflict / evolution / transitive
    rule TEXT,                   -- shared_neighbor / co_occurrence / type_pattern (仅 synthesis)
    started_at TEXT NOT NULL,
    completed_at TEXT,
    edges_created INTEGER,
    edges_verified INTEGER,
    entities_scanned INTEGER,
    llm_calls INTEGER,
    tokens_used INTEGER,
    status TEXT DEFAULT 'running'
);
```

---

## 6. LLM 配置

复用现有 `nexus_env_vars()` 机制：

- `follow_agent`（默认）：维护/推理任务用 Agent 的模型
- `custom`：单独配置，适用于希望用便宜模型做维护的场景（如 DeepSeek 批量去重）

维护任务默认用 `follow_agent` 模式，优先使用 Agent 配置的模型。如果用户希望降低维护成本，可以在设置中切到 `custom` 模式配一个便宜模型（如 DeepSeek-V3）。

推理任务（冲突检测、观点总结、合成验证）的 prompt 复杂度高于维护任务（选择题 vs 总结题），token 消耗更大。默认也走 `follow_agent`，但建议用户在数据量增长后切 `custom` 配廉价模型。

---

## 7. 前端触发入口

设置页 "知识引擎" section 增加：

```
┌──────────────────────────────────────────────────────────┐
│ 知识引擎 — 维护                                           │
│                                                          │
│ 上次整理: 2026-05-31 03:00                               │
│ 实体总数: 451    低质量: 413   孤岛: 384   疑似重复: 3      │
│                                                          │
│ [去重检查] [质量评分] [完整整理] [首次整理]                   │
├──────────────────────────────────────────────────────────┤
│ 知识引擎 — 推理                                           │
│                                                          │
│ 合成边: 47   待验证: 23   冲突: 2   社区: 5               │
│                                                          │
│ [合成推理] [PageRank] [社区检测] [冲突扫描]                 │
└──────────────────────────────────────────────────────────┘
```

---

## 8. 能力检测测试

### 8.1 测试架构

```
测试目录:
src-tauri/
├─ tests/
│   ├─ common/mod.rs              # 测试夹具: 临时 DB + 示例数据 + Mock LLM
│   ├─ nexus_ingestion_test.rs    # 入库 + 提取 + 去重
│   ├─ nexus_query_test.rs        # 搜索 + 路径 + 邻居 + 地图
│   ├─ nexus_maintenance_test.rs  # 去重 + 质量 + 孤岛 + 分类 + 过期 + 迁移修复
│   ├─ nexus_synthesis_test.rs    # 3 条合成规则
│   ├─ nexus_graph_intel_test.rs  # PageRank + Louvain + 因果链
│   ├─ nexus_evolution_test.rs    # 传递推理 + 冲突 + 观点
│   ├─ nexus_api_test.rs          # HTTP 端点集成测试
│   └─ nexus_token_test.rs        # Token 预算控制验证

python/
├─ tests/
│   ├─ test_extract_service.py    # extract_service.py 各模式
│   ├─ test_file_tools.py         # PDF/DOCX/PPTX/XLSX 文本提取
│   ├─ test_maintain_scripts.py   # maintain_dedup/quality/cleanup
│   └─ test_infer_service.py      # infer_service.py 推理验证
```

所有 Rust 测试用 `cargo test`，Python 测试用 `pytest`，模拟 LLM 调用不消耗真实 token。

---

### 8.2 数据入库测试

**目标**: 验证从各种来源提取实体/关系的正确性。

| 测试项 | 输入 | 期望输出 | 验证方法 |
|--------|------|---------|---------|
| Wiki 文件提取 | `wiki/AI.md` 含 `[[深度学习]]` 和 `[[神经网络]]` | 2 个实体 + 1 条 wikilink 关系 | `extract_entities_from_text_local()` 返回 >= 2 实体 |
| 聊天对话提取 | "今天讨论了 Transformer 和 Attention 机制" | 2 个实体（名词提取） | `nexus_store(text, "chat")` 异步完成 |
| 画布 JSON | `{"nodes": [...], "edges": [...]}` | 实体/关系 1:1 映射 | `nexus_store_canvas()` 节点数 == JSON nodes 数 |
| PDF 文档 | `report.pdf` 含文本 | 文本提取成功 → LLM 摘要 | `file_tools.py extract_pdf` 返回非空 |
| DOCX 文档 | `notes.docx` 含表格 | 文本 + 表格内容 | `file_tools.py extract_docx` 含表格文本 |
| 图片描述 | `photo.png` | base64 → 多模态 LLM → 描述文本 | `nexus_describe_images()` 生成 .md |
| SHA256 去重 | 同一内容两次 `nexus_store()` | 第二次返回 "duplicate" | content_hash 匹配，日志显示跳过 |
| 本地正则回退 | LLM 不可用时的纯文本 | 5 种模式（wikilink/书名/身份/Dr./术语） | `extract_entities_from_text_local()` 返回非空 |
| 空内容 | 空字符串 / 纯标点 | 0 实体，不报错 | 返回空数组 |
| 超大文本 | >500KB 文本 | 截断不崩溃，最多 30 实体 | `MAX_ENTITIES_PER_DOC` 生效 |
| confidence 过滤 | LLM 返回 confidence < 0.4 的实体 | 被 `filter_and_deduplicate()` 丢弃 | 低置信度不出现在最终结果 |
| Levenshtein 去重 | "深度学习" 和 "深度學习"（相似度 > 85%） | 合并为 1 个实体 | 去重后只有 1 条 |
| 停用词过滤 | "的" "是" "在" 等词 | 不产生实体 | 纯停用词不出现 |
| heimdall_migrated 去重 | wiki 重提取产生实体 vs 已有 heimdall_migrated 同名实体 | 合并（更新 confidence，保留关系） | 新提取实体与旧迁移实体正确合并 |

---

### 8.3 基础查询测试

**目标**: 验证 5 个 Agent 可调用端点的正确性。

| 测试项 | 输入 | 期望输出 | 验证方法 |
|--------|------|---------|---------|
| FTS5 精确搜索 | `q="Transformer"` | 返回包含 Transformer 的实体 | 结果列表含目标实体 |
| FTS5 前缀搜索 | `q="Trans"` | 返回 Transformer/Transfer/Transpose | 前缀匹配生效 |
| FTS5 无结果回退 | `q="xyznotexist"` | LIKE 模糊匹配尝试，最终空数组 | 不报错，返回 `[]` |
| 搜索 + namespace 过滤 | `q="AI" namespace="ml"` | 只返回 namespace=ml 的 AI 相关 | 所有结果 namespace == "ml" |
| 搜索 + entity_type 过滤 | `q="AI" entity_type="concept"` | 只返回概念类型 | 所有结果 entity_type == "concept" |
| 实体详情 | `GET /nexus/entity/{id}` | 包含实体属性 + 入边列表 + 出边列表 | JSON 含 entity + in_edges + out_edges |
| 实体详情（无效 ID） | `GET /nexus/entity/nonexistent` | 404 + 错误消息 | 返回 404 |
| 最短路径（直连） | A→B 存在直接边 | 1 跳路径 | path.hops == 1 |
| 最短路径（多跳） | A→X→Y→B | 3 跳路径 | path.hops == 3，含中间节点 |
| 最短路径（无路径） | 两个不连通实体 | 空结果 | 返回 `[]` |
| 路径（模糊名称） | `from="深度学习" to="CNN"` | FTS5 先匹配名称 → BFS | 正确解析为 UUID |
| 邻居扩展（1 跳） | `GET /nexus/neighbors/{id}?hops=1` | 所有直接相邻实体 | 入边 + 出边邻居 |
| 邻居扩展（3 跳） | `GET /nexus/neighbors/{id}?hops=3` | BFS 展开 3 层 | 不超过 3 跳范围 |
| 知识地图 | `GET /nexus/map` | 领域统计 + Top5 + 子领域 + 跨域桥梁 | JSON 含 domains/entities/bridges |
| 并发请求 | 同时 10 个 search 请求 | 全部正确返回，无锁竞争 | SQLite busy 超时处理正常 |

---

### 8.4 维护能力测试

**目标**: 验证去重/质量/孤岛/分类/过期全部正确。

#### 去重合并

| 测试项 | 输入 | 期望输出 |
|--------|------|---------|
| 同名 + 同 type（高相似） | "Python" (concept) × 2 | LLM 判断 YES → 合并 |
| 同名 + 不同 type | "Python" (language) vs "Python" (animal) | LLM 判断 NO → 不合并 |
| 近名 + 同 namespace | "机器学习" vs "机器学习 " (尾空格) | SQL 相似度 > 0.8 → LLM 判断 YES |
| 翻译等价 | "Deep Learning" vs "深度学习" | FTS5 + 描述匹配 → LLM 判断 YES |
| 不同实体相似名 | "Java" vs "JavaScript" | LLM 判断 NO → 不合并 |
| 合并数据完整性 | 合并 A→B | A 的关系迁移到 B，A 的 source_count 累加到 B |
| 漏斗过滤效率 | 1000 实体去重 | SQL 候选 < 30 对 → 实际 LLM 调用 < 5 次 |

#### 质量评分

| 测试项 | 条件 | 期望质量等级 |
|--------|------|------------|
| 完整实体 | name + desc + ≥3 条关系 | A (高质量) |
| 缺描述 | name + ≥3 关系 但无 desc | B (中等) |
| 缺关系 | name + desc 但 0 关系 | C (低质量) |
| 缺描述 + 缺关系 | 仅 name | D (极低质量) |
| 描述过短 | desc < 20 字符 | 降一级 |
| pipeline 实体自动屏蔽 | 评分 D（仅 name）且 confidence < 0.4 | 标记 low_quality，建议隐藏 |

#### 孤岛清理

| 测试项 | 条件 | 期望动作 |
|--------|------|---------|
| 绝对孤岛 | 0 入边 + 0 出边 | 标记 archived |
| 单边孤岛 | 1 条边 | 不处理（有连接） |
| 核心实体误判 | PageRank top 10% | 即使孤岛也不归档（受保护） |
| 归档可恢复 | archived 实体 | 保留数据，`hidden=1`，不用 DELETE |
| 批量归档 | 100 个孤岛 | 每批 50，分 2 批 |

#### 分类纠错

| 测试项 | 输入 | 期望修正 |
|--------|------|---------|
| namespace 明显错误 | name="Python" type="person" namespace="programming" | type → "language" |
| 低置信度 + 孤立分类 | entity_type 使用次数 < 3 | 建议合并到相似高频类型 |
| 引号类型清理 | entity_type = `"concept"` (带双引号) | 去引号 → `concept`，合并到正确类型 |
| 引号类型保留 | entity_type = `"concept"` 且 concept 类型不存在 | 去引号 → `concept`（新建） |

#### 过期检测

| 测试项 | 条件 | 期望动作 |
|--------|------|---------|
| 源文件已删除 | source_path 对应的文件不存在 | 标记 stale |
| 长期未访问 | last_accessed > 90 天 | 标记 stale |
| 低置信度 + 过期 | confidence < 0.4 + stale | 自动隐藏 |
| 高置信度 + 过期 | confidence > 0.7 + stale | 只标记，不隐藏 |
| 手动创建的 | source_type = "manual" | 永不过期 |

#### 迁移修复

| 测试项 | 条件 | 期望动作 |
|--------|------|---------|
| heimdall_migrated + confidence=0 | source_type = heimdall_migrated, confidence = 0 | 标记 confidence_needs_fix=true |
| heimdall_migrated 有 source_path | source_path 指向存在的 wiki 文件 | 触发重提取，confidence 更新为 LLM 结果 |
| heimdall_migrated 无 source_path | source_path 为空或文件不存在 | 保留，等用户手动处理 |
| 批量修复 | 339 个 heimdall_migrated | 分批 50/批，分 7 批，记录进度 |

---

### 8.5 推理能力测试

#### 合成推理（3 条规则）

| 测试项 | 图结构 | 期望推断 |
|--------|--------|---------|
| 共享邻居触发 | A→X←B，X 度数 = 5 | A—[related_to]→B, c=0.25 |
| 共享邻居不触发（低度） | A→X←B，X 度数 = 1 | 不推断（枢纽度不够） |
| 共享邻居已存在边 | A→X←B，A-B 已直连 | 不重复创建 |
| 共现触发 | A、B 在 4 个不同 source_path 共现 | A—[co_occurs]→B, c=0.4 |
| 共现不触发 | A、B 共现仅 2 次 | 不推断（< 3 个 source） |
| 类型模式触发 | person 类型 15 个实体，12 个有 works_at | 记录模式 person→works_at |
| 类型模式不触发 | person 类型 10 个，仅 5 个有 works_at | 不记录（< 80%） |

#### 图智能

| 测试项 | 输入 | 期望输出 |
|--------|------|---------|
| PageRank 基本 | 简单链 A→B→C→D | B、C PR 最高（被引用多） |
| PageRank 收敛 | 100 实体随机图 | 20 次迭代内收敛（delta < 0.001） |
| PageRank 阻尼 | d=0.85 | 所有 PR > 0，无孤立 0 值 |
| Louvain 社区 | 两个簇 + 1 条跨簇边 | 检测出 2 个社区 |
| Louvain 单节点 | 1 实体 0 边 | 1 个社区（自身） |
| Louvain 模块度 | 明显簇结构 | Q > 0.3 |
| 因果链正向 | A—[cause]→B—[cause]→C | A 的后果链 = [B, C] |
| 因果链反向 | A—[cause]→B—[cause]→C | C 的根因链 = [B, A] |
| 因果链无结果 | 无边关系类型为 cause | 返回空 |

#### 知识演进

| 测试项 | 输入 | 期望输出 |
|--------|------|---------|
| 传递推理 is_a | A—[is_a]→B, B—[is_a]→C | A—[is_a]→C, c=min(c1,c2)×0.9 |
| 传递推理 part_of | A—[part_of]→B, B—[part_of]→C | A—[part_of]→C, c=min(c1,c2)×0.85 |
| 传递推理无传递性 | A—[works_at]→B, B—[works_at]→C | 不推断（非传递关系） |
| 冲突检测互斥边 | A—[supports]→X, A—[opposes]→X | 标记冲突 |
| 冲突检测非互斥 | A—[related_to]→X, A—[mentions]→X | 不冲突 |
| 冲突降低置信度 | 确认冲突 | 两条边置信度 ×0.5 |
| 观点演化（跨度 > 7 天） | 同一实体 3 个时间段描述 | LLM 生成时间线 |
| 观点演化（跨度不足） | 同一实体 2 天内 2 条描述 | 不触发 |

#### 推理验证

| 测试项 | 输入 | 期望输出 |
|--------|------|---------|
| LLM 批量验证 | 10 条合成边 | ~800 tokens，返回 YES/NO × 10 |
| 验证通过 | LLM 回复 YES | 置信度 0.25 → 0.7，source_type → verified |
| 验证拒绝 | LLM 回复 NO | 边删除 |
| 批量边界 | 0 条待验证 | 空操作 |
| 分批处理 | 35 条待验证 | 分 4 批（10+10+10+5） |
| Token 预算控制 | 预算 1000 tokens/次 | 第 2 批时预算用尽 → 标记 pending，下周期继续 |

---

### 8.6 API 端点集成测试

**目标**: 验证 HTTP 端点完整请求-响应周期。

```
测试场景:

1. 端到端提取链路:
   GET /nexus/maintain/health   (确认 :18643 在线)
   → 上传 wiki/AI.md 文件
   → 文件监控触发 nexus_extract_from_file
   → GET /nexus/search?q=AI  (确认实体入库)
   → GET /nexus/entity/{id}   (确认详情可查)

2. 维护链路:
   GET /nexus/maintain/status  (初始状态)
   → POST /nexus/maintain/dedup  (触发去重)
   → GET /nexus/maintain/status  (确认 completed)
   → 查询 cache_maintenance_log  (确认日志)

3. 推理链路:
   POST /nexus/synthesis/run  (触发合成)
   → GET /nexus/synthesis/status  (确认边数)
   → POST /nexus/synthesis/verify  (验证合成边)
   → GET /nexus/synthesis/status  (确认待验证归零)

4. 演化链路:
   选择实体 → GET /nexus/evolution/{id}  (查看观点时间线)
   → 确认返回按时间排序的描述变化

5. 错误处理:
   GET /nexus/entity/   (400: 缺少 id)
   POST /nexus/maintain/dedup 无 LLM 配置  (503: LLM 不可用)
   并发 POST /nexus/synthesis/run × 2  (409: 已有任务运行中)

6. 端口冲突回退:
   :18643 被占用 → 尝试 :18644 → :18645 → 写入 nexus_port 文件
```

---

### 8.7 Token 控制验证

**目标**: 确保所有 LLM 调用符合预算。

| 验证项 | 预算 | 测试方法 |
|--------|------|---------|
| 去重单次调用 | < 200 input + 1 output | Mock LLM 记录 token 数 |
| 去重全量 | < 5000 tokens | 1000 实体全量去重，统计 LLM 调用总 token |
| 每周期最大 LLM 调用 | ≤ 10 次 | 计数，第 11 次应被拒绝 |
| 每周期最大 token 预算 | ≤ 20000 | 累计 token，超预算时标记 pending |
| 推理验证单批 | < 1000 tokens | 10 条边输入 + 10 个 YES/NO 输出 |
| 冲突检测 | < 500 tokens | 互斥边候选少，token 极低 |
| 观点演化 | < 1000 tokens/实体 | 描述截断 + 限制时间段数 |
| 0 LLM 调用（规则任务） | 0 | 质量评分、孤岛清理、PageRank、社区检测、合成推理 |
| 预算耗尽恢复 | pending → 下周期继续 | 未完成任务在下一周期自动捡起 |

---

### 8.8 性能基准

**目标**: 确保大数据量下性能可接受。

| 场景 | 数据量 | 时间限制 | 内存限制 |
|------|--------|---------|---------|
| Wiki 元数据扫描 | 500 个 .md 文件（文件发现 + stale 检测，不含 LLM 提取） | < 30 秒 | < 200MB |
| FTS5 搜索 | 10000 实体 | < 50ms | - |
| BFS 路径查找 | 10000 实体, 4 跳 | < 500ms | - |
| 知识地图生成 | 5000 实体 | < 2 秒 | - |
| PageRank | 5000 实体, 20000 边 | < 5 秒 | - |
| Louvain 社区 | 5000 实体 | < 10 秒 | - |
| 质量扫描 | 10000 实体 | < 1 秒 | - |
| 孤岛检测 | 10000 实体 | < 100ms | - |
| 合成推理（3 规则） | 5000 实体 | < 3 秒 | - |
| 文件去重（SHA256） | 1000 文件 | < 1 秒 | - |
| 并发读取 | 50 并发 search | 无超时 | 无死锁 |
| 迁移重提取（单文件） | 1 个 source_path 重提取（extract_service.py） | < 3 秒 | - |
| 迁移重提取（批量） | 50 个文件/批 | < 150 秒 | - |

---

### 8.9 测试数据准备

```
测试夹具 (test fixtures):

1. 最小知识图谱 (10 实体, 15 关系):
   entities: [AI, ML, DL, CNN, RNN, Transformer, NLP, CV, Python, TensorFlow]
   relations: [AI→ML, ML→DL, DL→CNN, DL→RNN, DL→Transformer, AI→NLP, AI→CV, ...]

2. 孤岛测试集 (5 孤岛 + 5 正常):
   5 个无连接实体 + 5 个有连接实体

3. 去重测试集 (15 对):
   5 对同名同 type → 应合并
   5 对同名不同 type → 不应合并
   5 对近名 → 候选

4. 冲突测试集:
   A—[supports]→X
   A—[opposes]→X
   应检测到冲突

5. 传递测试集:
   A—[is_a]→B—[is_a]→C
   应推断 A—[is_a]→C

6. 大规模测试集 (5000 实体, 20000 关系):
   随机生成，用于性能基准

7. 多源类型测试集:
   chat / wiki / canvas / upload_doc / upload_image / manual / heimdall_migrated 各 5 条
```

---

### 8.10 测试命令

```bash
# Rust 单元 + 集成测试
cargo test nexus_                        # 所有 nexus_ 前缀的测试
cargo test nexus_ingestion               # 入库测试
cargo test nexus_query                   # 查询测试
cargo test nexus_maintenance             # 维护测试
cargo test nexus_synthesis               # 合成推理测试
cargo test nexus_graph_intel             # 图智能测试
cargo test nexus_evolution               # 知识演进测试
cargo test nexus_api                     # API 集成测试
cargo test nexus_token                   # Token 控制测试

# Python 测试
cd src-tauri/src/services
pytest tests/test_extract_service.py -v  # 提取服务各模式
pytest tests/test_file_tools.py -v       # 文件工具
pytest tests/test_maintain_scripts.py -v # 维护脚本
pytest tests/test_infer_service.py -v    # 推理验证脚本

# 性能基准
cargo bench nexus                        # Rust benchmark
pytest tests/ -k "perf" --benchmark      # Python benchmark

# Mock LLM 模式（不消耗 token）
NEXUS_MOCK_LLM=1 cargo test nexus_      # Rust mock
NEXUS_MOCK_LLM=1 pytest tests/           # Python mock

# 全量回归
cargo test && pytest tests/              # 全部测试
```
