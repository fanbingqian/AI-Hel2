# Nexus 知识引擎完整方案

> 合并「知识提取改进」「知识提取与图谱渲染 v2」「维护操作 v2 重构」三个方案，整合所有确认内容和修改意见。

---

## 一、大模型身份认知

### 大模型知道自己是谁

```
你是 Nexus 知识引擎的核心推理模块。

你的工作是：将用户的知识（文档、对话、笔记）转化为可长期检索、
可跨文档关联、可演化的结构化知识图谱。

你不是在"提取关键词"——你是在为用户构建他自己的知识维基。
你提取的每一个实体，未来都可能被用户搜索、被其他文档引用、
被图谱渲染出来供人探索。

所以：
  - 不要提取你不想在知识维基里看到的东西
  - 不要创造用户搜索不到的名称
  - 不要用模糊的分类让实体失去意义
  - 你构建的是用户的知识资产，不是临时的数据
  - 命名实体也要评估置信度——如果名称只是列表中一闪而过、没有上下文支撑，不要提取
  - 时间实体要有历史意义或叙事价值——"下周三开会"不是知识，"2008年金融危机"才是
  - 信息实体不足 3 个就如实返回 1 个或 2 个——不要为了提高数量而降低质量
  - 置信度低于 0.5 的实体一律不提取
```

### 大模型知道知识引擎的结构

```
知识引擎有三层结构：

第一层：文档层
  每个文档（.md / .canvas / 聊天记录）是一个独立的文档实体。
  文档是一等公民。所有知识从文档中提取。

第二层：命名实体层
  文档中明确提到的地名、组织名、人名、自然名。
  这些是被文档"锚定"的事实——它们在现实世界中有明确的指代。

第三层：信息实体层
  从文档中提取的概念、项目、工具、术语。
  这些是用户知识的"原子"——独立存在、可被跨文档关联。

三层之间的关系：
  文档 --contains--> 命名实体
  文档 --contains--> 信息实体
  文档 --extends/refers/related_to--> 其他文档
  实体 --uses/depends_on/...--> 其他实体
```

### 大模型知道自己的任务边界

```
你能做的:
  ✅ 从文档中提取实体和关系
  ✅ 判断实体之间的语义关联
  ✅ 评估实体的信息价值和独特性
  ✅ 发现文档之间的引用/扩展/主题关联

你不能做的:
  ❌ 修改文档原文
  ❌ 删除用户的数据
  ❌ 创造不存在的实体名称
  ❌ 在没有依据的情况下推断关系
```

---

## 二、文档实体提取

### 文档作为一等公民

```
每个文档在知识图谱中是一个独立的实体节点:

  entity_type = "document" 或 "__file__"
  id = "doc:{文件路径}" 或 UUID v5(文件路径)
  name = 文件名（去扩展名）
  description = LLM 生成的一句话摘要
  source_file = 原始文件路径
  confidence = 1.0（确定性实体，不需要猜测）
```

### 文档提取流程

```
新文档写入 wiki/ 目录后:

Step 1: 创建文档实体
  INSERT INTO cache_entities (id, name, entity_type="document", ...)
  → 无论内容是否为空，文件存在即创建

Step 2: 提取命名实体
  扫描全文 → 识别地名/组织名/人名/自然名
  → 每个命名实体创建节点
  → 创建 contains 关系: 文档 → 命名实体
  → 无数量限制

Step 3: LLM 提取信息实体
  调用大模型，按四步检查筛选
  → 信息密度 Top 3 + 分散度 Top 3 + 独特性 Top 3
  → 去重合并
  → 创建 contains 关系: 文档 → 信息实体

Step 4: 提取实体间关系
  LLM 分析已提取实体之间的关联
  → uses / depends_on / contains / located_in / creates

Step 5: 提取文档间关系
  LLM 分析本文档与已有文档的关系
  → extends / refers / related_to / precedes
  → 写入 cache_relations (from=docA, to=docB)
```

### 文档节点与其他节点的区别

| 属性 | 文档节点 | 命名实体节点 | 信息实体节点 |
|------|---------|------------|------------|
| entity_type | `document` / `__file__` | `location`/`organization`/`person`/`natural_feature` | 自由描述 |
| 是否必须 | ✅ 文件存在即创建 | ✅ 识别到就创建 | ⚠️ 通过四步检查筛选 |
| 孤岛判定 | 无关系即孤岛 | 无关系即孤岛 | 无关系即孤岛 |
| 删除规则 | 文档删除 → 可级联隐藏关联实体 | 独立判断 | 独立判断 |
| source_file | 自身路径 | 来源文档路径 | 来源文档路径 |
| confidence | 1.0 | 0.7-0.9 | 0.5-0.9 |

---

## 三、知识提取：四步检查框架

### 提取优先级

```
第一优先：文档自身 → 每个文件一个实体节点（type: document）

第二优先：命名实体 → 地名/组织名/人名/自然名/时间名
  - 识别就提取，理论上不限量
  - 但必须有置信度评估（0.0-1.0）
  - 置信度 < 0.5 的命名实体 → 不提取
  - 防止地理词典类文档提取出成百上千个无意义地名

第三优先：信息实体 → 通过四步检查筛选
  - 每步最多 3 个（不是必须 3 个）
  - 不足 3 个不强行凑数——宁缺毋滥
  - 置信度 < 0.5 的信息实体 → 不提取

第四优先：关系提取 → 实体间关系 + 文档间关系
```

### 置信度在两级实体中的作用

```
命名实体置信度:
  0.9+ — 明确提到且上下文验证（如"巴厘岛位于印度尼西亚"）
  0.7+ — 明确提到但上下文简单（如行程列表中出现的"金巴兰湾"）
  0.5+ — 文本中提到但无上下文验证
  <0.5 — 不提取（可能是误识别或无关提及）

信息实体置信度:
  0.9+ — 核心概念、贯穿全文、用户特有
  0.7+ — 具体名称、清晰定义
  0.5+ — 有信息量但不够独立
  <0.5 — 不提取
```

### 置信度理由字段

每个实体必须输出 `confidence_reason` ——大模型要解释**为什么给这个分数**。

```
LLM 自评置信度的可靠性依赖"给出理由"这一步。
SaySelf (EMNLP 2024) 证明：LLM 在输出置信度的同时生成自反思理由，
校准误差显著降低。ConFix (2024) 进一步证明：事实级别的理由可以
用来自我修正低置信度输出。

所以方案要求：不能只报数字，必须报理由。
```

输出 JSON 包含 `confidence_reason`：

```json
{
  "name": "巴厘岛",
  "type": "location",
  "confidence": 0.9,
  "confidence_reason": "文档核心目的地，全文提及23次，贯穿行程始终，是文档的锚点"
}
```

### 四步检查

#### 检查 1：信息密度（Top 3）

```
判断：这个实体是否具体、可独立存在？
  - 去掉它，文段还完整吗？
  - 它携带了多少信息量？
  
  ✅ "知识架构一体化方案" — 具体、独立、信息量大
  ❌ "容器化" — 太泛、可被更具体实体替代
```

#### 检查 2：分散度（Top 3）

```
判断：它在文档中均匀出现还是只在一处提及？
  - 分散出现 → 核心主题，confidence +0.2
  - 集中一处 → 局部细节，confidence 不变
```

#### 检查 3：独特性（Top 3）

```
判断：行业通用术语还是用户特有概念？
  - 通用词（Docker、Python）→ 可提取但标记为通用
  - 独特词 → 标记为用户知识，confidence +0.1
```

#### 检查 4：关联性

```
两个方面:
  A. 文档与文档的关系（LLM 判断后写入）
  B. 实体与原文档和其他文档的关系
```

### 命名实体类型约束

| 类型 | type 值 | 示例 | 提取条件 |
|------|---------|------|---------|
| 地名 | `location` | 巴厘岛、金巴兰湾 | 置信度 ≥ 0.5 |
| 企业/组织 | `organization` | 长实集团、阿雅娜度假酒店 | 置信度 ≥ 0.5 |
| 人名 | `person` | 李嘉诚 | 置信度 ≥ 0.5 |
| 自然景观 | `natural_feature` | Rock Bar、京打马尼火山 | 置信度 ≥ 0.5 |
| 时间 | `time` | 2024年、90年代、冷战时期 | 置信度 ≥ 0.5 |

### 时间实体的特殊说明

```
时间实体提取规则:
  - 具有历史意义或叙事价值的时间 → 提取
    例: "90年代香港股市"、"2008年金融危机"
  - 纯日程时间 → 不提取
    例: "2024年5月12日下午3点开会"
  - 文档中的关键时间节点 → 提取
    例: "巴厘岛行程第3天"、"冷战结束后"

时间实体置信度:
  0.9+ — 具有明确历史/叙事意义的时间
  0.7+ — 文档结构中的时间节点
  0.5+ — 提及但需上下文验证
  <0.5 — 纯日程或无关时间 → 不提取

防止时间提取泛滥:
  - 一个文档中最多提取 5 个时间实体
  - 超过时只取置信度最高的 5 个
```

---

## 四、图谱渲染：两层级视图

### 默认视图：文档关系网（社区折叠）

```
启动时默认显示:
  - 所有文档节点（type: document/__file__）
  - 文档之间有关系的 → 连线
  - 文档之间无关系的 → 独立显示（孤岛文档不算孤岛）
  - 不显示文档内部的实体
```

### 展开视图：文档内部知识

```
点击/双击文档节点展开:
  - 该文档节点（居中、高亮）
  - 该文档提取的命名实体（地名/组织/人名/自然名）
  - 信息实体（密度Top3 + 分散度Top3 + 独特性Top3 去重合并）
  - 所有关系线

再次点击/双击 → 收起回到文档网络视图
```

### 配置面板开关

| 开关 | 效果 |
|------|------|
| **社区折叠**（默认开启） | 全局折叠 → 只有文档节点和文档间连线 |
| **社区折叠** 关闭 | 全局展开 → 所有文档+实体+关系 |

单个文档的双击操作独立于全局开关。

### 推断实体显示

```
传递推理产生的新实体:
  - 以灰色节点显示（区别于白色文档节点和彩色普通节点）
  - 节点无关联文档（source_file = null）
  - 鼠标悬停显示 "推断实体，无源文档"

UI 开关「推断实体可新建文档」:
  - 开启时 → 点击灰色推断实体 → 弹出 "为此实体创建文档" 按钮
  - 关闭时 → 灰色实体只读，不可点击创建
```

---

## 五、文档关系：文档间跳跃

### LLM 写入文档关系

```
提取完成后，LLM 分析当前文档与其他已知文档的关系:

  关系类型:
    extends    — A 是 B 的延伸/详细方案
    refers     — A 引用了 B
    related_to — A 和 B 主题相关
    precedes   — A 是 B 的前置/依赖

写入: cache_relations (from=docA_id, to=docB_id, type=关系)
```

### 文档间跳跃

```
图谱中:
  文档 A → 连线 → 文档 B（关系: extends）
  
  用户点击文档 A 和 B 之间的连线:
    → 显示关系说明: "A 是 B 的详细方案"
    → 显示 "跳转到文档 B" 按钮
    → 点击 → 展开文档 B 的内部知识视图
```

---

## 六、维护操作：6 个操作组

### 操作组 A：知识库健康检查

```
合并: 质量评分 + 修复迁移

流程:
  1. 实体分级（A/B/C/D）
  2. 文档节点不参与孤岛判定
  3. 类型引号清理
  4. D级 + confidence < 0.4 → hidden=1

不调 LLM，纯 SQL 规则
```

> **注意**：孤岛清理已取消。文档节点即使无关系也保留显示。

### 操作组 B：实体去重与合并

```
流程:
  1. Blocking: 字符相似度预分块
  2. Matching: LLM 批量判断是否重复（跨类型语义匹配）
  3. Merge: confidence ≥ 0.95 自动合并
           0.85-0.95 创建 SAME_AS 边标记审核
           < 0.85 保留
  4. Canonicalization: 选最长名称，旧名入 aliases

调 LLM
```

### 操作组 C：文档归类

```
增量模式: 扫描 wiki 根目录未分类文件 → LLM 建议文件夹 → 移动文件
全量模式: 遍历整个 wiki 树 → LLM 分类 → 移动文件

能力:
  - ✅ 可移动文档到已有文件夹
  - ✅ 可新建文件夹（LLM 建议文件夹名，不存在则创建）
  - ✅ 同名文件自动追加序号

最多 20 个文件/次，调 LLM
```

### 操作组 D：图谱结构分析

```
PageRank:
  d=0.85, max 100 轮迭代
  分别计算文档级和实体级重要性
  不调 LLM

社区检测:
  Louvain 贪婪算法，max 20 轮
  结果用于图谱社区折叠
  不调 LLM
```

### 操作组 E：关系推导与验证

```
传递推理:
  1. 规则扫描: 找 A→B→C 传递链（纯 SQL，不调 LLM）
  2. LLM 确定新实体: entity_type + name + description + confidence + confidence_reason
  3. 公式计算初始置信度: min(w1, w2) × 0.9, 上限 0.5
  4. LLM 审核置信度: 判断公式得出的置信度是否合理，给出 confidence_reason
     - 传递链可靠 → 保持公式计算的置信度
     - 传递链可疑 → 降低置信度，在 reason 中说明原因
  5. 推断边标记 inferred=1
  6. 推断出的新实体以灰色节点显示
  7. 循环迭代直到收敛（max 10 轮）

验证合成边:
  对 inferred=1 的边分批提交 LLM 验证
  确认保留 / 拒绝删除
  调 LLM
```

### 操作组 F：冲突与矛盾检测

```
显式矛盾: 检查预定义互斥对 + SQL 自连接
循环依赖: BFS 检测有向环
语义矛盾: LLM 批量审核

阶段 1+2 不调 LLM，阶段 3 调 LLM
```

---

## 七、各操作的大模型角色定义

每个需要调 LLM 的操作，都必须让大模型清楚知道：**我是谁、我在干什么、我的输出标准是什么**。

---

### A+. 命名实体与信息实体提取

```
## 角色
你是 Nexus 知识引擎的实体提取器。

你的任务是从文档中提取两类实体：命名实体和信息实体。

## 命名实体提取规则

### 类型限定
只提取以下 5 种类型:
  location       — 地名
  organization  — 企业/组织名
  person        — 人名
  natural_feature — 自然景观名
  time          — 具有历史/叙事意义的时间

### 提取条件
- 每个实体必须给出 confidence（0.0-1.0）
- 每个实体必须给出 confidence_reason（一句话解释为什么给这个分数）
- confidence < 0.5 的实体不提取

### 时间实体特殊规则
- 具有历史意义或叙事价值的时间 → 提取
  例: "90年代香港股市"、"2008年金融危机"
- 纯日程时间 → 不提取
  例: "2024年5月12日下午3点开会"
- 每个文档最多 5 个时间实体，超过取 confidence 最高的 5 个

### 数量控制
- 理论上不限量，但有置信度门槛
- 防止地理词典类文档提取出成百上千个无意义地名
- 如果文档中同类型实体超过 20 个，只取 confidence 最高的 20 个

## 信息实体提取规则

### 四步筛选
对非命名实体的候选实体，按以下顺序评估:

1. 信息密度: 是否具体、可独立存在？
2. 分散度: 贯穿全文还是局部提及？
3. 独特性: 用户特有术语还是行业通用词？
4. 关联性: 是否可以和其他实体形成关系？

### 数量控制
- 每步最多取 3 个，不足不凑数
- 宁缺毋滥——不要为填满 3 个而降低标准
- confidence < 0.5 的信息实体不提取

### 置信度分级
0.9+ — 核心概念、贯穿全文、用户特有 → confidence_reason 必须说明为什么是核心
0.7+ — 具体名称、清晰定义 → confidence_reason 说明具体性和定义来源
0.5+ — 有信息量但不够独立 → confidence_reason 说明为什么仍然值得提取
<0.5 — 不提取

## 输出 JSON
{
  "entities": [
    {
      "name": "巴厘岛",
      "type": "location",
      "confidence": 0.9,
      "confidence_reason": "文档核心目的地，全文提及23次，贯穿行程始终"
    }
  ],
  "relations": [...]
}
```

---

### B. 实体去重与合并

```
## 角色
你是 Nexus 知识引擎的去重审核员。

你的任务是：判断两个知识实体是否为"同一个东西"，防止知识图谱中
出现重复节点。

## 判断标准
请按以下维度逐一评估：

1. 名称等价性：
   - 两个名称是否指向同一个事物？
   - 允许翻译差异（"长实集团" vs "Cheung Kong Holdings"）
   - 允许缩写差异（"AI" vs "Artificial Intelligence"）
   - 不允许模糊相似（"知识引擎" vs "搜索引擎"不是同一个）

2. 描述一致性：
   - 两个描述是否在说同一件事？
   - 如果描述指向不同方面（同一人物的不同角色），不是重复

3. 类型兼容性：
   - person 和 organization 不能是重复
   - concept 和 tool 可能是重复（如果描述一致）
   - concept 和 project 可能是重复（如果名称和描述都匹配）

4. 来源一致性：
   - 来自同一文档？可能是同一实体被提取了两次
   - 来自不同文档？需要更严格的判断

## 输出
{
  "match": true/false,
  "confidence": 0.0-1.0,
  "reason": "一句话说明判断依据"
}

## 置信度参考
0.95+  — 确定无疑（同名+同描述+同类型）
0.85+  — 高度可能（名称等价+描述高度相似，类型不同但兼容）
0.70+  — 可能重复（部分匹配，需要更多信息）
<0.70  — 不是重复
```

---

### C. 文档归类

```
## 角色
你是 Nexus 知识引擎的文档管理员。

你的任务是：阅读文档内容，判断它属于什么主题领域，并建议
最佳的文件夹归档位置。

## 判断标准

1. 主题识别：
   - 文档主要讨论什么？（技术/旅行/金融/历史/个人...）
   - 如果有多个主题，选最主要的

2. 文件夹命名：
   - 用简洁的中文名词（2-4个字）
   - 如：技术架构、旅行攻略、投资分析、人物研究
   - 不要创建过于细分的文件夹（如"巴厘岛SPA体验"）
   - 如果已有合适的文件夹，使用已有名称

3. 归档规则：
   - 同类文档归入同一文件夹
   - 如果文档数量少（<3个），放根目录即可
   - 不要创建只有1个文件的文件夹

## 输出
{
  "folder": "建议的文件夹名（如不存在将自动创建）",
  "title": "文档标题（用于文件重命名）",
  "tags": ["标签1", "标签2"]
}
```

---

### E. 验证合成边

```
## 角色
你是 Nexus 知识引擎的推理审核员。

你的任务是：审核由规则引擎自动推断的关系边是否合理，
拒绝错误的推断，保证知识图谱的准确性。

## 判断标准

对于给定的推断边 A --[type]--> C：

1. 传递链验证：
   - 已知 A→B→C，推断 A→C。这个传递链是否成立？
   - 关系类型是否具有传递性？（is_a 有传递性，uses 没有）

2. 语义合理性：
   - A 和 C 之间确实存在 type 关系吗？
   - 即使传递链正确，结论是否荒谬？（狗 is_a 动物，猫 is_a 动物 → 狗 is_a 猫？）

3. 置信度评估：
   - 中间节点 B 的可靠性？（权重、来源）
   - 传递链长度？（2 跳 > 3 跳 > 4 跳）

## 输出
{
  "valid": true/false,
  "reason": "一句话说明审核理由"
}

## 重要提示
你只审核推断边是否合理。不要修改边的类型或方向。
不确定时，倾向于拒绝（宁可少，不可错）。
```

---

### F. 语义矛盾检测

```
## 角色
你是 Nexus 知识引擎的矛盾检测员。

你的任务是：检查知识图谱中是否存在逻辑矛盾——
同一实体对同一目标同时拥有相互冲突的关系。

## 判断标准

对于给定的关系对 A--[type1]-->B 和 A--[type2]-->B：

1. 语义冲突：
   - type1 和 type2 是否在逻辑上相互矛盾？
   - 例：supports vs opposes, agrees vs disagrees
   - 不一定非要是预定义的对立词

2. 上下文判断：
   - 是否可能是不同时间、不同语境下的关系？
   - 是否可能是互补关系而非矛盾？（"批评"和"支持"可能共存）

3. 严重程度：
   - 直接矛盾（supports+opposes）→ 高严重度
   - 语义冲突（recommends+criticizes）→ 中严重度
   - 疑似冲突（uses+avoids）→ 低严重度

## 输出
{
  "conflict": true/false,
  "severity": "high/medium/low",
  "reason": "一句话说明矛盾原因"
}
```

---

---

## 八、传递推理的 LLM 命名

### 为什么需要 LLM 参与传递推理

传递推理的核心步骤是"给定 A→B→C 的传递链，确定 C 的名称和类型"。
这需要语义理解——LLM 比规则更适合这个任务。

### 学术支撑

**CATS: Context-aware Inductive Knowledge Graph Completion (2024)**
Yang et al., arXiv:2410.16803

核心发现：LLM 配合 prompt-guided reasoning 可以在知识图谱中评估推断三元组的合理性。论文提出的 subgraph reasoning 模块通过选择相关推理路径和邻居事实来评估推断的置信度——这和我们的传递推理场景一致。实验在 transductive/inductive/few-shot 三种设置下均有 7.2% 的 MRR 提升。

**In-Context Learning with Topological Information for KGC (ICML 2024 SPIGM Workshop)**
Papasotiriou et al.

证明将图谱拓扑结构（节点间的替代路径）注入 LLM 上下文，配合 Chain-of-Thought 推理，可以显著提升推断三元组的质量。论文明确指出：路径式推理可以自然地转化为链式思考提示。这直接支持我们的设计——将 A→B→C 传递链作为 LLM 的推理上下文。

**Towards Semantically Enriched Embeddings for KGC (2024)**
Alam et al., *Neurosymbolic AI*

指出传递语义约束（如 is_a 层级、part_of 传递性）在当前研究中未得到充分解决，是 KGC 领域的开放问题。我们的方案正是针对这个缺口。

### LLM 在传递推理中的角色

```
输入: 传递链 A --[type]--> B --[type]--> C
      + A 的描述 + B 的描述

LLM 任务:
  1. 确定实体名称: C 应该叫什么？
     - 如果 C 在图中已存在 → 使用已有名称
     - 如果 C 不存在 → 基于 A 和 B 推断名称
     例: A="巴厘岛"(location), B="印度尼西亚"(location)
         → C="印度尼西亚"（已存在，复用）

  2. 确定实体类型: C 的 entity_type？
     - 从 A 或 B 的类型继承（取更具体的）
     - 如果 A 是 location，B 是 country → C 也是 location

  3. 生成描述: "通过传递推理从 {A} → {B} → {C} 推断"

  4. 置信度评估: 这个推断链的可靠程度？
     - 关系类型是否具有传递性？
     - 中间节点 B 的可靠性？
```

---

## 九、文档提取标记系统

### 设计思路

每个文档有一个提取标记。有标记 = 已提取，无标记 = 需要提取。

```
标记存在 → 跳过提取
标记消失 → 需要重新提取
```

### 实现方式

利用已有的 `cache_content_index` 表：

```
表结构（已有）:
  source_path  TEXT     -- 文件路径
  source_type  TEXT     -- 文件类型
  content_hash TEXT     -- 内容哈希
  extracted_at TEXT     -- 提取时间
  entity_count INTEGER -- 提取到的实体数
```

### 标记生命周期

```
1. 新文档写入 wiki/
   → cache_content_index 中无记录 → 需要提取 → 提取后写入记录

2. 文档内容被修改（用户编辑后保存）
   → content_hash 变化 → 标记失效 → 下次扫描时需要重新提取

3. 文档被移动到新路径（文档归类）
   → source_path 变化 → 旧记录失效 → UPDATE source_path = new_path
   → 同时 content_hash 不变 → 不需要重新提取
   → 只需要更新 source_file 字段即可

4. 文档被删除
   → DELETE FROM cache_content_index WHERE source_path = ...

5. 新增文档
   → cache_content_index 中无记录 → 需要提取
```

### 提取触发逻辑

```
扫描 wiki/ 目录:
  for each file:
    old_hash = cache_content_index 中该路径的 content_hash
    new_hash = SHA256(文件内容)
    
    if old_hash IS NULL:
      → 新文件，提取
    else if old_hash != new_hash:
      → 文件被修改过，重新提取
    else:
      → 内容未变，跳过
```

### 与维护操作的关系

```
文档归类 (操作组 C):
  - 移动文件 → 自动更新 source_path
  - 不改变 content_hash → 不触发重新提取
  - 源文件路径变了 → 自动更新关联实体的 source_file

手动触发:
  - 用户可以在前端点击 "重新提取" 按钮
  - 强制清除该文件的 content_hash → 下次扫描时重新提取
```

---

## 十、隐藏实体清理

### 确认删除的 hidden=1 场景

| 位置 | 场景 | 处理 |
|------|------|------|
| L6323 质量评分 D 级 | 四步检查在提取时过滤，不产生 D 级 | **删除此代码** |
| L6377 孤岛清理 | 操作已取消 | **删除此代码** |
| L6403 过期文件 | 改为文档删除时级联删除 | **删除此代码** |
| L6634 去重后清孤岛 | 与方案矛盾，去重不应产生新孤岛 | **删除此代码** |

### 保留的 hidden=1 场景

| 位置 | 场景 |
|------|------|
| L5962 | 用户手动删除实体 |
| L6050 | 批量操作 hide |
| L6618 | 去重合并后的旧实体 |

---

## 十一、数据冲突处理

```
问题: nexus_store（聊天提取）用 INSERT OR REPLACE，会删除旧行 →
      新建行 hidden=0 → 之前标记 hidden=1 的实体被复活

修复: INSERT OR REPLACE → INSERT ... ON CONFLICT(id) DO UPDATE
      SET description=..., confidence=..., 但 hidden 值不变

其他提取路径（文件提取、LLM 提取）用 ON CONFLICT DO NOTHING，无此问题
```

### 隐藏实体机制缩减

```
方案实施后，隐藏实体（hidden=1）只保留两种情况:
  1. 去重合并后的旧实体 → hidden=1（关系链可追溯）
  2. 用户手动隐藏 → hidden=1

以下场景不再产生隐藏实体:
  - 质量评分 D 级 → 四步检查在提取时已过滤，不会产生 D 级实体
  - 孤岛清理 → 已取消此操作
  - 过期清理 → 文档删除时级联删除，不标记隐藏
```

---

## 十二、文档生命周期

### 文档删除级联处理

```
删除文档时的连锁操作:

Step 1: 删除文档实体
  DELETE FROM cache_entities WHERE id = 'doc:{path}'

Step 2: 级联删除文档产生的关联实体
  命名实体（该文档提取的 location/organization/person/natural_feature）
    → 这些实体只属于这个文档 → 删除
  信息实体（该文档提取的 concept/project/tool 等）
    → 这些实体只属于这个文档 → 删除

Step 3: 取消跨文档关联边
  其他文档的实体与本文档实体之间的关联边
    → 不是删除实体，只删除这些边
    → 其他文档的实体仍然存在，只是失去了一部分关联

Step 4: 取消文档间关系边
  本文档与其他文档之间的 extends/refers/related_to/precedes 边
    → 全部删除
```

### 文档归类后行为

```
归类改变路径后的自动操作:

1. 自动更新 source_file
   归类完成后自动执行:
   UPDATE cache_entities SET source_file = ?1 WHERE source_file = ?2
   (new_path, old_path)

2. 触发重新提取
   提取策略:
     - 首次使用 → 对全部文档进行提取
     - 后续使用 → 只对新增文档提取
     - 手动归类/手动调整的文档 → 对该文档重新提取
   
   前端操作:
     「文档归类(增量)」→ 只扫描根目录新增文件
     「文档归类(全量)」→ 遍历全部文档
     归类完成后自动对该文档触发重新提取
```

---

## 十三、传递推理产出物

### 产生实体节点 + 关联边

```
传递推理不仅产生关系边（A→C），还产生实体节点。

当推断出 A→C 关系时:
  1. 如果 C 是已有实体 → 只创建推断边（标记 inferred=1）
  2. 如果 C 不存在 → 创建新的推断实体节点 + 创建推断边

新推断实体:
  entity_type: 由 LLM 确定（基于 A 和 B 的 entity_type 推断）
  name: 由 LLM 确定（基于上下文命名）
  description: 由 LLM 生成（"通过传递推理从 A→B→C 推断"）
  source_file: null（无源文档）
  inferred: true
  confidence: min(w1, w2) × 0.9, 上限 0.5
  color: #888888（灰色）
  _sphereRadius: 与普通实体相同

LLM 确定 entity_type 和 name 的原则:
  - entity_type 继承自 A 或 B 的类型（取更具体的）
  - name 使用传递链中最明确的名称
  - 如果 A→B→C 中 B 是中间节点，C 的名称应反映最终实体而非中间节点
```

### Obsidian 对比

```
Obsidian 没有"无文档的实体"概念。
  - Obsidian 中一切皆笔记（文档），没有独立的实体概念
  - 孤岛 = 零入链 + 零出链的笔记
  - 无法直接表达"推理产生的知识"

我们的方案是增强:
  - 推断实体以灰色显示，区别于文档实体（白色）和命名实体（彩色）
  - 用户可选择将推断实体"升级"为文档（通过 UI 开关创建文档）
```

### 推断边视觉规范

```
推断边（inferred=1）与普通边视觉效果相同。
不做虚线或灰度区分——传递推理的置信度已经体现在边的 weight 上，
视觉区分会让图谱变得杂乱。
```

---

## 十四、跨文档展开行为

### 默认状态

```
图谱默认显示: 社区折叠视图（只显示文档节点 + 文档间连线）
```

### 展开单个文档

```
双击文档 A → 展开:
  显示: 文档 A + A 的命名实体 + A 的信息实体 + 它们之间的关系线
  不显示: 文档 B 或其他文档的实体（即使它们与 A 的实体有关联）
```

### 展开多个文档

```
双击文档 A → 展开 A
双击文档 B → 展开 B（A 保持展开）

此时显示:
  - 文档 A + A 的所有关联实体
  - 文档 B + B 的所有关联实体
  - A 的实体 ↔ B 的实体之间有连线 → 显示（跨文档关联可见）
  - A 的实体 ↔ B 之间有连线但目标不在展开范围 → 不显示

示例:
  文档A中包含"巴厘岛"(location)，文档B中包含"金巴兰湾"(location)
  两个实体之间有 located_in 关系 → 连线显示
  但文档C中的"阿雅娜"(organization)不会显示（C未展开）
```

---

## 十五、UI 布局

### 知识引擎页面结构

```
┌─ 服务状态 ───────────────────────────────────────────┐
│  服务运行中 · 端口 18643    [重新检测]                  │
├─ 大模型配置 ─────────────────────────────────────────┐
│  模式: 跟随Agent / 自定义                              │
│  提供商 | API Key | 模型 | [验证连接] [重置]            │
├─ 知识库状态 ─────────────────────────────────────────┐
│  实体总数 | 上次整理 | 低质量 | 孤岛 | 疑似重复          │
├─ 维护操作 ───────────────────────────────────────────┐
│  A 知识库健康检查    [运行检查]                        │
│  B 实体去重与合并    [运行去重]                        │
│  C 文档归类          [增量归类] [全量归类]              │
│  D 图谱结构分析      [PageRank] [社区检测]             │
│  E 关系推导与验证    [传递推理] [验证合成边]            │
│  F 冲突与矛盾检测    [扫描冲突]                        │
└──────────────────────────────────────────────────────┘
```

### 图谱配置面板 v2 对照

#### 筛选区

| 当前设置 | v2 状态 | 处理 |
|---------|---------|------|
| 搜索 | ✅ 不变 | 搜索文档名/实体名 |
| 孤立节点 | ⚠️ 逻辑调整 | 保留 — 但文档节点无关系也算孤岛 |
| 社区折叠 | ✅ 不变 | 已实现 |
| 最低重要性 | ❌ | **删除** — 四步检查替代了重要性过滤 |
| 探索深度 | ❌ | **删除** — 文档展开替代了深度探索 |
| 颜色分组 | ✅ 不变 | 用户自定义颜色规则 |
| **推断实体可新建文档** | 🆕 | **新增** — 点击灰色推断节点弹出创建文档按钮 |
| **最小连接数** | ❌ | **删除** — 类型字段 `minDegree` 无面板控件 |

#### 外观区

| 当前设置 | v2 状态 |
|---------|---------|
| 箭头、属性圆环、文本透明度、节点大小、连线粗细、边线透明度 | ✅ 全部保留 |

#### 力度区

| 当前设置 | v2 状态 |
|---------|---------|
| 向心力 0-1、节点排斥力 0-20、相连节点吸引力 0-1、连线长度 30-500、拖拽引力 1-15 | ✅ 全部保留 |

#### 类型清理

| 字段 | 处理 |
|------|------|
| `minImportance` | **删除** — 类型 + 默认值 + Store 持久化 |
| `explorationDepth` | **删除** — 类型 + 默认值 + Store 持久化 |
| `minDegree` | **删除** — 类型 + 默认值（面板无控件） |

---

### confidence_reason 前端显示位置

| 场景 | 显示方式 | 示例 |
|------|---------|------|
| **图谱实体详情面板** | 点击节点 → 右侧详情浮层，置信度后附理由 | `置信度: 0.9 — 文档核心目的地，全文提及23次` |
| **实体列表** | 鼠标悬停实体名 → tooltip 显示理由 | `来自文档"xxx.md"，信息密度评估: 贯穿全文` |
| **维护操作结果** | 每条 detail 附理由 | `合并: "人工智能"(concept) ← "AI"(tool) — LLM判断为同一实体` |
| **传递推理结果** | 灰色节点详情面板 | `推断实体 — 通过 A is_a B is_a C 推断，LLM审核通过` |

### 维护操作结果的 reason 字段

所有调 LLM 的维护操作，输出必须包含 `reason`：

```
知识库健康检查: 不调LLM → 不需要 reason
实体去重与合并:  每条 merge/same_as 附 reason ✅
文档归类:        每条 classify 附 reason ✅
验证合成边:      每条 valid/rejected 附 reason ✅
语义矛盾检测:    每条 conflict 附 reason ✅
```

---

## 十六、关键数据结构

### 文档间关系

```json
{
  "from_id": "doc:巴厘岛旅游.md",
  "to_id": "doc:科莫多旅游.md",
  "relation_type": "extends",
  "label": "巴厘岛行程延伸至科莫多",
  "confidence": 0.8
}
```

### 推断实体

```json
{
  "id": "inferred:uuid",
  "name": "推断实体名",
  "entity_type": "inferred",
  "description": "通过 A→B→C 传递推理得到",
  "confidence": 0.5,
  "source_file": null,
  "inferred": true,
  "color": "#888888"
}
```

---

## 十七、实施路线图

### 优先级说明

| 级别 | 含义 |
|------|------|
| 🔴 P0 | 方案执行前必须先修——不修影响后续所有工作的正确性 |
| 🟡 P1 | 方案核心内容——提取质量、UI结构 |
| 🟢 P2 | 方案新功能——传递推理升级、文档生命周期 |

---

### 🔴 P0-1: cache_content_index 显式建表

**问题**: `cache_content_index` 表在 `nexus_store`、`extract_entities` 等多处被 `INSERT/UPDATE`，但 `init_cache_tables` 中从未 `CREATE TABLE`。依赖 SQLite 在首次 INSERT 时自动创建——如果列类型推断错误或并发写入，会导致数据损坏。

**修复文件**: `knowledge_service.rs`

**修复内容**:
1. 在 `init_cache_tables` 中增加 `CREATE TABLE IF NOT EXISTS cache_content_index` 语句
2. 确认列定义与所有 INSERT 语句的列对齐
3. 确认已有数据的表不会因 CREATE IF NOT EXISTS 受影响

**验证**: 
- 删除 `knowledge_cache.db`，重启应用，检查表是否自动创建
- 提取一个文档，检查 `cache_content_index` 是否有记录
- 再次提取同一文档，检查 `content_hash` 是否阻止了重复提取

---

### 🔴 P0-2: nexus_store INSERT OR REPLACE 修复

**问题**: `knowledge_service.rs:2606` 行，聊天提取路径使用 `INSERT OR REPLACE INTO cache_entities`。当实体已存在且 `hidden=1` 时，REPLACE 会删除旧行再插入新行，新行 `hidden` 取默认值 0——导致已隐藏的实体被复活。

**影响范围**: 质量评分隐藏的 D 级实体、去重合并隐藏的旧实体、用户手动隐藏的实体——只要在聊天中提及，就会被复活。

**修复文件**: `knowledge_service.rs`

**修复内容**:
1. 将 `INSERT OR REPLACE` 改为 `INSERT ... ON CONFLICT(id) DO UPDATE SET`
2. UPDATE 子句列出需要更新的字段（description, confidence, updated_at），不包含 hidden
3. 保留 `hidden` 的当前值不变

**变更前后对比**:
```sql
-- 修复前
INSERT OR REPLACE INTO cache_entities (id, name, ..., hidden) VALUES (...)

-- 修复后  
INSERT INTO cache_entities (id, name, ...) VALUES (...)
ON CONFLICT(id) DO UPDATE SET
  description = excluded.description,
  confidence = excluded.confidence,
  updated_at = excluded.updated_at
-- hidden 不在 UPDATE 列表中 → 保持原值不变
```

**验证**:
- 手动将某个实体设为 `hidden=1`（SQLite 直接操作）
- 在聊天中提及该实体名称
- 检查该实体仍为 `hidden=1`

---

### 🟡 P1-1: 提取 prompt 重写

**问题**: `extract_service.py` 中三个 prompt 仍是旧版：
- 提取 prompt: "你是知识筛选器" → 应为 "你是 Nexus 知识引擎的核心推理模块"
- 分类 prompt: "你是文档分类助手" → 应加入身份认知
- 无 `confidence_reason` 字段
- 无四步检查框架
- 无命名实体类型约束和时间实体规则

**修复文件**: `extract_service.py`

**修复内容**:
1. 重写 `build_prompt()` —— 身份认知 + 三层结构 + 任务边界 + 四步检查 + 置信度分级 + confidence_reason
2. 重写 `build_classify_prompt()` —— 加入身份认知
3. 重写 `build_summarize_prompt()` —— 加入身份认知
4. 输出 JSON Schema 增加 `confidence_reason` 必填字段
5. 命名实体类型约束：location/organization/person/natural_feature/time 五种
6. 时间实体数量上限 5 个
7. 同类型实体超过 20 个时只取置信度最高的 20 个
8. 信息实体每步最多 3 个，不足不凑数

**依赖**: P0 两项必须先完成

**验证**:
- 提取一个包含地名、人名、时间的文档，检查输出 JSON 中每个 entity 是否有 `confidence_reason`
- 提取一个地理词典类文档，检查是否被数量控制限制在合理范围
- 提取一个信息密度低的文档，检查是否返回少于 3 个实体

---

### 🟡 P1-2: 类型清理 + UI 合并

**问题**:
1. `GraphSettings2D` 类型中 `minImportance`/`explorationDepth`/`minDegree` 三种字段已确认删除
2. 知识引擎维护操作前端 UI 仍是 10 项旧布局

**修复文件**: `knowledge.ts`、`knowledgeStore.ts`、`GraphSettingsPanel.tsx`、`SettingsPage.tsx`、`ForceGraph2DWrapper.tsx`

**修复内容**:

A. 类型清理:
1. `knowledge.ts`: 删除 `minImportance`/`explorationDepth`/`minDegree` 三个字段（接口 + 默认值）
2. `knowledgeStore.ts`: 删除 `loadGraphSettings` 中三个字段的持久化代码
3. `ForceGraph2DWrapper.tsx`: 删除 `buildGraphData` 调用中的 `minImportance`/`minDegree`/`explorationDepth` 参数

B. UI 合并:
1. `GraphSettingsPanel.tsx`: 删除「最低重要性」「探索深度」两个 slider
2. `GraphSettingsPanel.tsx`: 筛选区新增「推断实体可新建文档」toggle
3. `SettingsPage.tsx`: 维护操作从 10 项合并为 6 组

**验证**:
- TypeScript 编译无报错
- 图谱配置面板打开正常，删除的 slider 不出现
- 知识引擎维护操作显示 6 组

---

### 🟢 P2-1: 传递推理 LLM 命名

**问题**: 当前传递推理纯 SQL 执行，只产生边不产生实体。方案要求：
- 产生新实体节点（灰色）+ 关联边
- entity_type + name + confidence_reason 由 LLM 确定
- 置信度 = 公式计算 + LLM 审核

**修复文件**: `knowledge_service.rs`、`extract_service.py`（或新建 LLM 调用）、`graphAdapter.ts`、`ForceGraph2DWrapper.tsx`

**修复内容**:
1. 传递推理函数增加 LLM 调用步骤（确定实体名/类型/置信度理由）
2. 新实体插入 `cache_entities`（source_file=null, inferred=true）
3. `graphAdapter.ts` 识别 `inferred=true` 实体，color 设为 `#888888`
4. `ForceGraph2DWrapper.tsx` SVG 渲染灰色节点
5. 面板新增「推断实体可新建文档」开关（P1-2 已加 toggle，此处实现交互）

**验证**:
- 运行传递推理后，图谱中出现灰色节点
- 灰色节点详情面板显示推断理由
- 打开开关后点击灰色节点弹出"创建文档"按钮

---

### 🟢 P2-2: 文档删除级联处理

**问题**: 删除 wiki 文档后，其关联实体仍留在数据库——变成"幽灵实体"。

**修复文件**: `knowledge_service.rs`

**修复内容**:
1. 新增 `cascade_delete_document(file_path)` 函数
2. 删除文档实体
3. 删除文档产生的命名实体和信息实体（级联）
4. 取消跨文档关联边（其他文档实体 ↔ 本文档实体之间的边）
5. 取消文档间关系边（本文档与其他文档的 extends/refers 等边）
6. 删除 `cache_content_index` 中的提取记录

**验证**:
- 删除一个已提取的 wiki 文档
- 检查其关联实体是否被删除
- 检查其他文档的实体是否保留（只删除了边）

---

## 十八、自检测试方案

### 测试原则

```
每完成一个 Phase → 运行对应测试 → 全部通过才能进入下一 Phase
测试分为: 编译检查 / 单元验证 / 数据完整性 / UI 交互
```

---

### Phase 1 测试（P0 修复 + P1 清理）

#### 测试 1.1: 编译检查

```bash
# Rust
cd src-tauri && cargo check
# TypeScript
npx tsc -b --noEmit
```

**通过标准**: 零 error，warning 仅限预存项

#### 测试 1.2: 数据库完整性

```sql
-- 验证 cache_content_index 表存在
SELECT name FROM sqlite_master WHERE type='table' AND name='cache_content_index';
-- 预期: 1 行

-- 验证表结构正确
PRAGMA table_info(cache_content_index);
-- 预期: source_path, source_type, content_hash, extracted_at, entity_count 五列
```

**通过标准**: 表存在 + 列完整

#### 测试 1.3: hidden 实体复活防护

```bash
# 1. 手动 SQL: UPDATE cache_entities SET hidden=1 WHERE id='test-entity'
# 2. 在聊天中输入包含 "test-entity" 的消息
# 3. 检查: SELECT hidden FROM cache_entities WHERE id='test-entity'
```

**通过标准**: hidden 仍为 1，未被复活

#### 测试 1.4: 类型清理编译

```bash
npx tsc -b --noEmit
```

**通过标准**: `minImportance`/`explorationDepth`/`minDegree` 三个字段从类型定义中移除后编译通过

#### 测试 1.5: UI 控件检查

```
打开图谱配置面板 → 筛选区:
  ✅ 搜索框存在
  ✅ 孤立节点 toggle 存在
  ✅ 社区折叠 toggle 存在
  ✅ 推断实体可新建文档 toggle 存在（新增）
  ❌ 最低重要性 slider 不存在（已删除）
  ❌ 探索深度 slider 不存在（已删除）
```

**通过标准**: 新增项出现 + 删除项不出现

#### 测试 1.6: 维护操作 UI

```
打开知识引擎 → 维护操作:
  ✅ A 知识库健康检查 — [运行检查] 按钮
  ✅ B 实体去重与合并 — [运行去重] 按钮
  ✅ C 文档归类 — [增量归类] [全量归类] 按钮
  ✅ D 图谱结构分析 — [PageRank] [社区检测] 按钮
  ✅ E 关系推导与验证 — [传递推理] [验证合成边] 按钮
  ✅ F 冲突与矛盾检测 — [扫描冲突] 按钮
```

**通过标准**: 6 组全部显示，旧版 10 项不再出现

---

### Phase 2 测试（提取升级 + 文档标记）

#### 测试 2.1: 编译检查

```bash
cd src-tauri && cargo check && npx tsc -b --noEmit
```

#### 测试 2.2: 提取 prompt 输出格式

```bash
# 用测试文本调用 extract_service.py
echo "巴厘岛是印度尼西亚的一个省，位于金巴兰湾。2024年我们去了阿雅娜度假酒店。" | \
  python extract_service.py --mode text 2>&1

# 检查输出 JSON
```

**通过标准**:
1. 每个 entity 对象包含 `confidence_reason` 字段 ✅
2. entity_type 只出现 location/organization/person/natural_feature/time 五种 ✅
3. 无 entity_type 为 `location/bar` 之类的复合类型 ✅
4. confidence < 0.5 的实体不出现在输出中 ✅

#### 测试 2.3: 命名实体置信度

```
测试文档: 包含 50 个地名的地理词典文件
预期: 只提取 confidence ≥ 0.5 的地名，不超过 20 个（同类型上限）
```

**通过标准**: 提取数量 ≤ 20，每个都有 confidence_reason

#### 测试 2.4: 信息实体数量控制

```
测试文档: 信息密度低的短文本（如"今天天气不错"）
预期: 返回 0-2 个信息实体，不强行凑 3 个
```

**通过标准**: 实体数 ≤ 2

#### 测试 2.5: 时间实体提取

```
测试文档: 包含多个时间引用的历史文章
预期: 
  - "2008年金融危机" → 提取 (confidence ≥ 0.7)
  - "下周三开会" → 不提取
  - 最多 5 个时间实体
```

**通过标准**: 只有历史/叙事时间被提取，最多 5 个

#### 测试 2.6: 文档提取标记

```bash
# 1. 首次提取文档 A → cache_content_index 写入记录
# 2. 再次提取文档 A（内容未变）→ content_hash 一致 → 跳过
# 3. 修改文档 A 内容 → 再次提取 → content_hash 不一致 → 重新提取
```

**通过标准**: 
- 首次提取后有记录 ✅
- 内容不变时跳过（entity_count=0） ✅
- 内容变化后重新提取 ✅

#### 测试 2.7: 文档归类后 source_file 更新

```bash
# 1. 文档 A 归类前 source_file = "wiki/A.md"
# 2. 运行文档归类 → 移动到 "wiki/技术/A.md"
# 3. 检查 A 的实体 source_file = "wiki/技术/A.md"
```

**通过标准**: 归类后所有关联实体的 source_file 同步更新

---

### Phase 3 测试（图谱交互）

#### 测试 3.1: 编译检查

```bash
npx tsc -b --noEmit
```

#### 测试 3.2: 推断实体灰色渲染

```
运行传递推理 → 打开图谱:
  ✅ 新产生的推断实体显示为灰色（#888888）
  ✅ 普通文档实体为白色
  ✅ 命名实体为彩色（按 entity_type 着色）
  ✅ 推断边的视觉效果与普通边一致
```

**通过标准**: 三种节点颜色可区分

#### 测试 3.3: 推断实体新建文档

```
图谱中:
  1. 打开「推断实体可新建文档」开关
  2. 点击灰色推断实体 → 弹出 "为此实体创建文档" 按钮
  3. 点击按钮 → 创建新文档 → 灰色节点变为普通实体节点
  4. 关闭开关 → 点击灰色实体无反应
```

**通过标准**: 开关控制交互行为正确

#### 测试 3.4: 社区折叠全局开关

```
图谱配置:
  1. 默认「社区折叠」开启 → 只显示文档节点 + 文档间连线
  2. 关闭「社区折叠」→ 全部文档展开，显示内部实体
  3. 单击某个文档节点 → 右侧显示文档详情
  4. 双击文档节点 → 独立展开/折叠（不受全局开关影响）
```

**通过标准**: 全局开关 + 独立双击均正确

#### 测试 3.5: 跨文档展开

```
图谱中:
  1. 双击文档 A → 显示 A 的实体（不显示 B 的实体）
  2. 双击文档 B → A 保持展开，B 展开
  3. A 的实体 ↔ B 的实体有连线 → 显示
  4. 双击 A 收起 → A 的实体消失，B 保持展开
```

**通过标准**: 多文档展开/收起互不干扰，跨文档连线正确

---

### Phase 4 测试（维护升级）

#### 测试 4.1: 编译检查

```bash
cd src-tauri && cargo check
```

#### 测试 4.2: 跨类型语义去重

```
测试数据: 创建两个同名不同 type 的实体（如 "人工智能" concept + "人工智能" tool）
运行去重 → LLM 判断为同一实体 → 合并
```

**通过标准**: 跨类型重复被识别并合并

#### 测试 4.3: 循环依赖检测

```
测试数据: 创建 A→B→C→A 的依赖环
运行冲突扫描 → 检测到循环依赖 → 输出 warning
```

**通过标准**: 循环依赖被检测并报告

#### 测试 4.4: 文档删除级联

```bash
# 1. 文档 A 有 5 个关联实体 + 3 条与其他文档实体的连线
# 2. 删除文档 A
# 3. 检查:
#    - A 的 5 个关联实体是否被删除
#    - 其他文档实体之间的连线是否被删除（只删边，不删实体）
#    - 文档间关系边是否被删除
#    - cache_content_index 记录是否被删除
```

**通过标准**: 级联删除完整，其他文档实体不受影响

---

### 全量回归测试

全部 Phase 完成后执行：

```bash
# 1. 全量编译
cd src-tauri && cargo check && cd .. && npx tsc -b --noEmit

# 2. 数据库完整性
sqlite3 ~/.ai-hel2/knowledge_cache.db "PRAGMA integrity_check;"

# 3. 提取流程端到端
# 新建 wiki 文档 → 自动提取 → 检查实体数量和质量

# 4. 图谱渲染
# 打开图谱 → 社区折叠默认显示文档节点 → 展开文档查看实体

# 5. 维护操作
# 依次运行 6 个操作组 → 检查输出报告格式

# 6. UI 完整性
# 遍历所有设置面板 → 检查无死开关、无错位控件
```

---

## 十九、参考文献

- Guellil et al. "Entity Linking for English and Other Languages: A Survey." *KAIS*, 2024.
- Zhang et al. "Extraction and Evaluation of Knowledge Entities from Scientific Documents." *Scientometrics*, 2024.
- Zheng et al. "A Comprehensive Survey on Document-Level Information Extraction." *FuturED @ ACL*, 2024.
- Seitl et al. "Assessing the Quality of Information Extraction." *arXiv:2404.04068*, 2024.
- "The Rise of Semantic Entity Resolution" (2025) — LLM 驱动的跨类型实体匹配与合并
- "Building Production Knowledge Graphs" (FalkorDB, 2024) — 自动本体约束与唯一性检查
- "Entity Resolution and Deduplication" (Neo4j Agent Memory, 2025) — SAME_AS 边 + 连通分量合并
