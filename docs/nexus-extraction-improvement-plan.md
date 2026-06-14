# Nexus 知识提取改进方案

> 基于 2024 年学术研究，重构实体提取的思考框架和 Prompt

## 一、当前问题

| 问题 | 表现 | 根因 |
|------|------|------|
| 类型碎片化 | `location/bar`, `natural_attraction`, `experience/spa` | LLM 自由命名无约束 |
| concept 占比 46% | 技术术语、人物观点、项目名称全混在一起 | [[wikilink]] 解析硬编码为 concept |
| 重复实体 | 同名不同 type | LLM 不知道已有实体 |
| 关系稀疏 | 18 种关系，12 种只有 1 条边 | 关系提取无指导 |

## 二、学术研究支撑

### 核心发现：频率 ≠ 重要性

CMU Kernel Entity Salience Model (KESM) 指出实体提取不应依赖词频，而应基于信息价值。

### 最强预测因子（按优先级）

| 信号 | 含义 |
|------|------|
| 提及分散度 | 实体在文档不同段落中均匀出现 vs 集中一处 |
| 共指簇大小 | 同一实体被多种名称/代词指代的次数 |
| 语篇深度 | 实体在文档结构树中的位置（靠近根 = 更重要） |
| 位置 | 实体在文档前部的提及更有价值 |
| 主语性 | 实体是否作为句子的主语 |

### MINEA 质量评估方法 (arXiv 2024)

- 往文档里插入已知实体（needles），检查 LLM 能否抽到
- 同时测完整性（召回）和精确性（准确率）
- 实验证明 3-4 轮迭代后回报递减

### SIGIR 2025: 三元组 TF-IDF

一个实体的价值 = 它在本文档内的重要度 vs 它在背景语料中的普遍性:
- 常见词（Docker、Python）→ 提取但低置信度
- 独特术语（per-agent namespace）→ 高置信度

## 三、新的思考框架：四步检查

```
第一步：信息密度检查
  → 是否具体、可独立存在？
  → 去掉这个实体，文段还完整吗？

第二步：分散度检查
  → 是否贯穿全文还是局部提及？
  → 均匀分散 = 核心主题，集中一处 = 边缘细节

第三步：独特性检查
  → 是否用户特有的术语（vs 行业通用词）？
  → 通用词提取但降权，独特术语升权

第四步：关联性检查
  → 是否可以和其他实体形成关系？
  → 孤岛实体降低优先级，有关系的提取
```

## 四、Prompt 改造方向（待确认后执行）

```markdown
## 角色
你是知识架构师。从文档中提取有长期检索价值的知识实体。

## 判断流程
对每个候选实体，按顺序评估：

### 1. 信息密度
- 它是具体的事物还是泛泛的描述？
- 如果从文档中删除它，相关内容还完整吗？
- ✅ 具体、不可删除 → 提取
- ❌ 泛泛、可删除 → 跳过

### 2. 分散度
- 它在文档中均匀出现还是只在一处提及？
- 分散出现 → 核心主题，confidence +0.2
- 集中一处 → 局部细节，confidence 不变

### 3. 独特性
- 它是行业通用术语还是用户特有的概念？
- 通用词（如 Docker、Python）→ 可提取但标记为通用
- 独特词 → 标记为用户知识，confidence +0.1

### 4. 关联性
- 它是否可以和文档中的其他实体形成关系？
- 有关联 → 同时提取关系
- 孤立 → 降低优先级

## 规则
1. 最多 N 个实体，宁少勿滥
2. 类型命名用简洁名词，同类事物用一致的类型名
3. 不要创建复合类型（如 location/bar → 用 location）
```

## 五、参考文献

- Guellil et al. "Entity Linking for English and Other Languages: A Survey." *Knowledge and Information Systems*, 2024.
- Zhang et al. "An Editorial Note on Extraction and Evaluation of Knowledge Entities." *Scientometrics*, 2024.
- Zheng et al. "A Comprehensive Survey on Document-Level Information Extraction." *FuturED @ ACL*, 2024.
- Seitl et al. "Assessing the Quality of Information Extraction." *arXiv:2404.04068*, 2024.
- Arslan. "Business Insights Using RAG–LLMs." *Journal of Decision Systems*, 2024.
- Speck. "Knowledge Extraction for the Data Web." Doctoral Dissertation, Paderborn University, 2024.
