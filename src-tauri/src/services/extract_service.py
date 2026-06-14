"""
Nexus Knowledge Extraction Service.
Extracts entities and relations from text using LLM API.
Supports --mode text / document / image.

Usage:
  echo "text content" | python extract_service.py --mode text [--context "{...}"]

Config via environment variables (set by Rust caller):
  NEXUS_LLM_MODE       — "follow_agent" (default) | "custom"
  NEXUS_LLM_PROVIDER   — "anthropic" | "openai" | "deepseek" | "custom"
  NEXUS_LLM_MODEL      — model name
  NEXUS_LLM_API_KEY    — API key
  NEXUS_LLM_BASE_URL   — API base URL (optional, inferred from provider)
  NEXUS_MAX_ENTITIES   — max entities per extraction (default 10)
"""

import sys
import os
import json
import argparse
import hashlib
import urllib.request
import urllib.error

sys.stdout.reconfigure(encoding="utf-8")

# ── Stop words ──
STOP_WORDS = {
    "东西", "事情", "这个", "那个", "它们", "我们", "他们", "她们",
    "什么", "怎么", "哪里", "这里", "那里", "因为", "所以", "但是",
    "如果", "虽然", "可以", "需要", "应该", "能够", "可能", "已经",
    "没有", "知道", "觉得", "认为", "使用", "通过", "进行", "其他",
    "一些", "一下", "上面", "下面", "前面", "后面", "左右", "等等",
    "一个", "一种", "很多", "一般", "基本", "全部",
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


def load_custom_stop_words():
    """Load user stop words from ~/.ai-hel2/stop_words.txt."""
    home = os.environ.get("AI_HEL2_HOME", os.path.expanduser("~/.ai-hel2"))
    path = os.path.join(home, "stop_words.txt")
    if os.path.exists(path):
        try:
            with open(path, "r", encoding="utf-8") as f:
                for line in f:
                    word = line.strip()
                    if word and not word.startswith("#"):
                        STOP_WORDS.add(word)
        except Exception:
            pass


def get_llm_config():
    """Read LLM configuration from environment variables."""
    mode = os.environ.get("NEXUS_LLM_MODE", "follow_agent")
    provider = os.environ.get("NEXUS_LLM_PROVIDER", os.environ.get("LLM_PROVIDER", "anthropic"))
    model = os.environ.get("NEXUS_LLM_MODEL") or os.environ.get("LLM_MODEL") or "claude-sonnet-4-6"
    api_key = os.environ.get("NEXUS_LLM_API_KEY") or os.environ.get("ANTHROPIC_API_KEY") or ""
    base_url = os.environ.get("NEXUS_LLM_BASE_URL") or ""

    # Resolve base_url from provider if not explicitly set
    if not base_url:
        provider_urls = {
            "anthropic": "https://api.anthropic.com",
            "openai": "https://api.openai.com",
            "deepseek": "https://api.deepseek.com",
            "hermes_builtin": "http://127.0.0.1:18642",
        }
        base_url = provider_urls.get(provider, "https://api.anthropic.com")
    # Ensure /v1 suffix for OpenAI-compatible API calls
    if base_url and not base_url.rstrip("/").endswith("/v1"):
        base_url = base_url.rstrip("/") + "/v1"

    return {
        "mode": mode,
        "provider": provider,
        "model": model,
        "api_key": api_key,
        "base_url": base_url,
    }


def call_llm(messages, config):
    """Call LLM API (Anthropic or OpenAI-compatible) and return text response."""
    provider = config["provider"]
    api_key = config["api_key"]
    base_url = config["base_url"]
    model = config["model"]

    if not api_key and provider != "hermes_builtin":
        return None, "No API key configured"

    if provider == "anthropic":
        url = f"{base_url}/messages"
        body = json.dumps({
            "model": model,
            "max_tokens": 2048,
            "messages": messages,
        }).encode("utf-8")
        headers = {
            "x-api-key": api_key,
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
        }
    else:
        url = f"{base_url}/chat/completions"
        body = json.dumps({
            "model": model,
            "max_tokens": 2048,
            "messages": messages,
        }).encode("utf-8")
        headers = {
            "Authorization": f"Bearer {api_key}",
            "content-type": "application/json",
        }

    try:
        req = urllib.request.Request(url, data=body, headers=headers, method="POST")
        with urllib.request.urlopen(req, timeout=60) as resp:
            result = json.loads(resp.read().decode("utf-8"))

        if provider == "anthropic":
            content = result.get("content", [{}])
            return content[0].get("text", "") if content else "", None
        else:
            choices = result.get("choices", [{}])
            msg = choices[0].get("message", {}) if choices else {}
            return msg.get("content", ""), None
    except urllib.error.HTTPError as e:
        error_body = e.read().decode("utf-8")[:500] if e.fp else ""
        return None, f"HTTP {e.code}: {error_body}"
    except Exception as e:
        return None, str(e)


def build_prompt(text, context, mode, max_entities):
    """Build extraction prompt — Nexus v2 with identity, 4-step check, confidence_reason."""
    mode_instructions = {
        "text": "以下是一段文本，请提取其中的知识实体和关系。",
        "document": "以下是一份文档内容，请提取其中的知识实体和关系。",
        "image": "以下是一张图片的描述文本，请提取其中的知识实体和关系。",
    }

    context_block = ""
    if context:
        try:
            ctx_obj = json.loads(context)
            ctx_text = json.dumps(ctx_obj, ensure_ascii=False, indent=2)
            context_block = f"## 对话上下文\n{ctx_text}\n\n"
        except json.JSONDecodeError:
            context_block = f"## 上下文\n{context}\n\n"

    prompt = f"""## 角色
你是 Nexus 知识引擎的核心推理模块。

你的工作是：将用户的知识转化为可长期检索、可跨文档关联的结构化知识图谱。
你不是在"提取关键词"——你是在为用户构建他自己的知识维基。

知识引擎有三层结构：
  第一层 文档层 — 每个文档是独立实体
  第二层 命名实体层 — 地名/组织名/人名/自然名/时间名
  第三层 信息实体层 — 概念/项目/工具/术语

{context_block}## 待提取内容
{text}

## 命名实体提取（5 种固定类型，不限量但有置信度门槛）

只提取以下类型的命名实体：
  location         — 地名（如：巴厘岛、金巴兰湾）
  organization     — 企业/组织名（如：长实集团、阿雅娜度假酒店）
  person           — 人名（如：李嘉诚）
  natural_feature  — 自然景观名（如：Rock Bar岩石酒吧、京打马尼火山）
  time             — 时间（只提取有历史/叙事意义的，如：90年代、2008年；不提取纯日程）

命名实体规则：
  - 每个实体必须给出 confidence（0.0-1.0）和 confidence_reason（一句话解释）
  - confidence < 0.5 的命名实体不提取
  - 同类型实体超过 20 个时，只取 confidence 最高的 20 个
  - 时间实体每个文档最多 5 个

## 信息实体提取（自由类型，四步检查筛选）

对非命名实体的候选实体，按以下四步评估：

检查 1: 信息密度
  它是否具体、可独立存在？去掉它文段还完整吗？
  具体独立 → confidence 0.7+
  泛泛描述 → 跳过

检查 2: 分散度
  贯穿全文还是局部提及？
  贯穿全文 → confidence +0.1
  集中一处 → 不加分

检查 3: 独特性
  用户特有术语还是行业通用词？
  用户特有 → confidence +0.1
  行业通用 → 不加分

检查 4: 关联性
  是否可以和其他实体形成关系？
  可关联 → 同时提取关系
  孤立 → 降低优先级

信息实体规则：
  - 每步最多取 3 个，不足不凑数——宁缺毋滥
  - confidence < 0.5 不提取
  - 最多提取 {max_entities} 个信息实体

## 置信度分级标准（所有实体通用）

0.9 — 核心主题，贯穿全文，用户特有知识
0.7 — 具体名称，清晰定义
0.5 — 有信息量但不够独立
< 0.5 — 不提取

每个实体必须输出 confidence_reason，说明为什么给这个分数。
SaySelf (EMNLP 2024) 证明：输出理由能显著提升 LLM 自评置信度的可靠性。

## 输出 JSON（只输出 JSON）
{{
  "entities": [
    {{
      "name": "实体名",
      "type": "location/organization/person/natural_feature/time（命名实体）或自由描述（信息实体）",
      "description": "一句话描述",
      "confidence": 0.0,
      "confidence_reason": "一句话解释为什么给这个置信度",
      "properties": {{}}
    }}
  ],
  "relations": [
    {{
      "from": "实体A名",
      "type": "uses/depends_on/contains/located_in/creates",
      "to": "实体B名",
      "confidence": 0.0
    }}
  ]
}}

请提取："""

    return prompt


def levenshtein(a, b):
    """Compute Levenshtein distance between two strings."""
    if len(a) < len(b):
        return levenshtein(b, a)
    if len(b) == 0:
        return len(a)
    prev = range(len(b) + 1)
    for i, ca in enumerate(a):
        curr = [i + 1]
        for j, cb in enumerate(b):
            cost = 0 if ca.lower() == cb.lower() else 2 if ca != cb else 1
            curr.append(min(prev[j + 1] + 1, curr[j] + 1, prev[j] + cost))
        prev = curr
    return prev[-1]


def similarity(a, b):
    """Levenshtein similarity ratio (0.0-1.0)."""
    max_len = max(len(a), len(b))
    if max_len == 0:
        return 1.0
    return 1.0 - levenshtein(a, b) / max_len


def filter_and_deduplicate(entities, relations):
    """Apply 1.3 filter rules: confidence, stop words, name length, dedup."""
    filtered = []

    for e in entities:
        name = e.get("name", "").strip()
        confidence = e.get("confidence", 0.5)

        # Rule 1: confidence threshold
        if confidence < 0.4:
            continue

        # Rule 6: name length
        if len(name) < 2 or len(name) > 60:
            continue

        # Rule 2: stop words
        name_lower = name.lower()
        if name_lower in STOP_WORDS or name in STOP_WORDS:
            continue
        if any(sw.lower() == name_lower for sw in STOP_WORDS):
            continue

        # Rule 3-4: dedup by name similarity
        is_dup = False
        for existing in filtered:
            existing_name = existing.get("name", "")
            sim = similarity(name, existing_name)
            if sim > 0.85:
                # Keep the one with higher confidence
                if confidence > existing.get("confidence", 0):
                    existing["name"] = name
                    existing["confidence"] = confidence
                    existing["description"] = e.get("description", "") or existing.get("description", "")
                is_dup = True
                break

        if not is_dup:
            # Normalize fields
            e["name"] = name
            e["confidence"] = confidence
            filtered.append(e)

    # Dedup relations: same (from, type, to) with same entity
    seen_rels = set()
    unique_rels = []
    for r in relations:
        key = (r.get("from", "").lower(), r.get("type", "").lower(), r.get("to", "").lower())
        if key not in seen_rels:
            seen_rels.add(key)
            unique_rels.append(r)

    # Cap entity count
    max_ents = int(os.environ.get("NEXUS_MAX_ENTITIES", 10))
    return filtered[:max_ents], unique_rels


def extract_json_from_response(text):
    """Extract JSON object from LLM response (may have markdown fences)."""
    text = text.strip()

    # Try to find JSON block in markdown
    if "```json" in text:
        start = text.index("```json") + 7
        end = text.index("```", start) if "```" in text[start:] else len(text)
        text = text[start:end].strip()
    elif "```" in text:
        start = text.index("```") + 3
        end = text.index("```", start) if "```" in text[start:] else len(text)
        text = text[start:end].strip()

    # Try direct parse
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass

    # Try to find first { ... } block
    brace_start = text.find("{")
    brace_end = text.rfind("}")
    if brace_start >= 0 and brace_end > brace_start:
        try:
            return json.loads(text[brace_start:brace_end + 1])
        except json.JSONDecodeError:
            pass

    return {"entities": [], "relations": []}


# ── Summarization prompt ──

def build_classify_prompt(text, file_type, file_name, existing_dirs):
    """Build a classification prompt for auto-archiving documents."""
    dirs_list = json.dumps(existing_dirs, ensure_ascii=False) if existing_dirs else "[]"
    max_chars = 3000
    truncated = text[:max_chars]

    prompt = f"""## 角色
你是 Nexus 知识引擎的文档管理员。

你的任务是：阅读文档内容，判断它属于什么主题领域，并建议最佳的文件夹归档位置。

## 已有目录
{dirs_list}

## 文件名
{file_name}
类型: {file_type.upper()}

## 判断标准
1. 主题识别：文档主要讨论什么？（技术/旅行/金融/历史/个人...）
2. 文件夹命名：用简洁的中文名词（2-4个字），如：技术架构、旅行攻略、投资分析
3. 不要创建过于细分的文件夹，如果已有合适的文件夹则使用已有名称
4. 同类文档归入同一文件夹，不要创建只有1个文件的文件夹

## 要求
1. folder: 从已有目录中选择最合适的，如果都不合适则建议新目录名
2. title: 给文档起简洁中文标题（10字以内）
3. tags: 提取2-5个中文标签
4. 只输出 JSON，不要其他文字

## 内容
{truncated}

## 输出格式
{{"folder": "建议目录", "title": "文档标题", "tags": ["标签1", "标签2"]}}

请输出 JSON："""
    return prompt

def build_summarize_prompt(text, file_type, file_name):
    """Build a summarization prompt for documents."""
    # Truncate very long texts
    max_chars = 12000
    truncated = text[:max_chars]
    truncation_note = ""
    if len(text) > max_chars:
        truncation_note = f"\n(原文共 {len(text)} 字符，已截取前 {max_chars} 字符)"

    prompt = f"""## 角色
你是 Nexus 知识引擎的文档分析助手。

你的任务是：将文档内容总结为结构化的 Markdown，保留关键信息供知识图谱索引。

## 文件名
{file_name}

## 要求
1. 用 ## 标题作为文档标题
2. 提取关键要点，使用有序/无序列表
3. 保留重要数据、日期、人名、术语
4. 如果有表格数据，用 Markdown 表格呈现
5. 用 --- 分隔不同主题段落
6. 输出纯 Markdown，不要 JSON，不要代码块包裹
7. 控制在 500-2000 字

## 原文内容{truncation_note}
{truncated}

请输出 Markdown 摘要："""

    return prompt


def build_image_description_prompt(image_count, batch_title=""):
    """Build a prompt for image description."""
    title_line = f"## 图集标题\n请根据图片内容起一个简洁的中文标题，格式：`# 标题`\n"
    if batch_title:
        title_line = f"标题: {batch_title}\n"

    if image_count == 1:
        prompt = f"""## 角色
你是图片分析助手。请描述这张图片的内容。

## 要求
1. 以 `# 图片标题` 开头（根据内容起名）
2. 描述图片的主要内容和关键细节
3. 如果图片中有文字，请提取出来
4. 如果是图表/架构图，解释其含义
5. 控制在 200-500 字
6. 输出纯 Markdown

请描述："""
    else:
        prompt = f"""## 角色
你是图片分析助手。以下是同一批次上传的 {image_count} 张图片，请分析并整理。

## 要求
1. 以 `# 图集标题` 开头（根据所有图片的共同主题起名，格式：`# 图集标题 - 主题名`）
2. 先写一段总述（2-3句话概括这批图片的整体内容）
3. 用 `---` 分隔
4. 然后逐张图片用 `## 图片 N` 描述每张图的内容
5. 如果图片中有文字，请提取出来
6. 如果是图表/架构图，解释其含义
7. 每张图片控制在 150-300 字
8. 输出纯 Markdown，不要 JSON，不要代码块包裹

请描述："""
    return prompt


def call_llm_multimodal(messages, config):
    """Call multimodal LLM API with image content (base64 data URLs)."""
    provider = config["provider"]
    api_key = config["api_key"]
    base_url = config["base_url"]
    model = config["model"]

    if not api_key and provider != "hermes_builtin":
        return None, "No API key configured"

    try:
        if provider == "anthropic":
            url = f"{base_url}/messages"
            # Convert messages format for Anthropic: extract image data from content
            anthropic_content = []
            for msg in messages:
                if isinstance(msg.get("content"), list):
                    for block in msg["content"]:
                        if block["type"] == "text":
                            anthropic_content.append({"type": "text", "text": block["text"]})
                        elif block["type"] == "image_url":
                            # Parse data URL: data:image/png;base64,XXXX
                            data_url = block["image_url"]["url"]
                            header, b64data = data_url.split(",", 1)
                            media_type = header.split(":")[1].split(";")[0]
                            anthropic_content.append({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": b64data,
                                },
                            })
                else:
                    anthropic_content.append({"type": "text", "text": str(msg["content"])})

            body = json.dumps({
                "model": model,
                "max_tokens": 2048,
                "messages": [{"role": "user", "content": anthropic_content}],
            }).encode("utf-8")
            headers = {
                "x-api-key": api_key,
                "anthropic-version": "2023-06-01",
                "content-type": "application/json",
            }
        else:
            url = f"{base_url}/chat/completions"
            body = json.dumps({
                "model": model,
                "max_tokens": 2048,
                "messages": messages,
            }).encode("utf-8")
            headers = {
                "Authorization": f"Bearer {api_key}",
                "content-type": "application/json",
            }

        req = urllib.request.Request(url, data=body, headers=headers, method="POST")
        with urllib.request.urlopen(req, timeout=120) as resp:
            result = json.loads(resp.read().decode("utf-8"))

        if provider == "anthropic":
            content = result.get("content", [{}])
            return content[0].get("text", "") if content else "", None
        else:
            choices = result.get("choices", [{}])
            msg = choices[0].get("message", {}) if choices else {}
            return msg.get("content", ""), None
    except urllib.error.HTTPError as e:
        error_body = e.read().decode("utf-8")[:500] if e.fp else ""
        return None, f"HTTP {e.code}: {error_body}"
    except Exception as e:
        return None, str(e)


def main():
    parser = argparse.ArgumentParser(description="Nexus Knowledge Extraction Service")
    parser.add_argument("--mode", default="text",
                        help="Mode: text/document/image/summarize/describe_images")
    parser.add_argument("--context", default=None, help="Optional conversation context (JSON)")
    parser.add_argument("--file-type", default="document", help="File type for summarization")
    parser.add_argument("--file-name", default="file", help="Original file name for summarization")
    parser.add_argument("--image-count", default="1", help="Number of images for describe mode")
    parser.add_argument("--existing-dirs", default="[]", help="JSON list of existing directories")
    args = parser.parse_args()

    load_custom_stop_words()
    config = get_llm_config()

    # ── Classify mode ──
    if args.mode == "classify":
        raw = sys.stdin.buffer.read()
        text = raw.decode("utf-8").strip()
        if not text:
            print(json.dumps({"error": "no text provided"}))
            sys.exit(1)
        try:
            existing_dirs = json.loads(args.existing_dirs)
        except json.JSONDecodeError:
            existing_dirs = []
        print(f"[Nexus] Classify: {len(text)} chars, type={args.file_type}", file=sys.stderr)
        prompt = build_classify_prompt(text, args.file_type, args.file_name, existing_dirs)
        messages = [{"role": "user", "content": prompt}]
        response_text, error = call_llm(messages, config)
        if error:
            print(f"[Nexus] Classify error: {error}", file=sys.stderr)
            sys.exit(1)
        # Parse JSON from response
        result = extract_json_from_response(response_text)
        print(f"[Nexus] Classify: {result}", file=sys.stderr)
        sys.stdout.write(json.dumps(result, ensure_ascii=False))
        return

    # ── Summarize mode ──
    if args.mode == "summarize":
        raw = sys.stdin.buffer.read()
        text = raw.decode("utf-8").strip()
        if not text:
            print("")
            sys.exit(0)
        print(f"[Nexus] Summarize: {len(text)} chars, type={args.file_type}", file=sys.stderr)
        prompt = build_summarize_prompt(text, args.file_type, args.file_name)
        messages = [{"role": "user", "content": prompt}]
        response_text, error = call_llm(messages, config)
        if error:
            print(f"[Nexus] Summarize error: {error}", file=sys.stderr)
            sys.exit(1)
        print(f"[Nexus] Summary: {len(response_text)} chars", file=sys.stderr)
        sys.stdout.write(response_text)
        return

    # ── Describe images mode ──
    if args.mode == "describe_images":
        raw = sys.stdin.buffer.read()
        input_data = json.loads(raw.decode("utf-8"))
        images = input_data.get("images", [])  # list of base64 data URLs
        batch_title = input_data.get("title", "")
        if not images:
            print(json.dumps({"error": "no images provided"}))
            sys.exit(1)
        image_count = len(images)
        print(f"[Nexus] DescribeImages: {image_count} images", file=sys.stderr)

        text_prompt = build_image_description_prompt(image_count, batch_title)
        # Build multimodal message content
        content_blocks = [{"type": "text", "text": text_prompt}]
        for b64_url in images:
            content_blocks.append({
                "type": "image_url",
                "image_url": {"url": b64_url},
            })

        messages = [{"role": "user", "content": content_blocks}]
        response_text, error = call_llm_multimodal(messages, config)
        if error:
            print(f"[Nexus] DescribeImages error: {error}", file=sys.stderr)
            sys.exit(1)
        print(f"[Nexus] Description: {len(response_text)} chars", file=sys.stderr)
        sys.stdout.write(response_text)
        return

    # ── Original extraction mode (text/document/image) ──
    raw = sys.stdin.buffer.read()
    text = raw.decode("utf-8").strip()

    if not text:
        print(json.dumps({"entities": [], "relations": [], "error": "no text provided"}))
        sys.exit(0)

    max_entities = int(os.environ.get("NEXUS_MAX_ENTITIES", 10))

    print(f"[Nexus] Mode: {args.mode}, Text: {len(text)} chars, Model: {config['model']}", file=sys.stderr)
    print(f"[Nexus] Config: provider={config['provider']}, base_url={config['base_url']}", file=sys.stderr)

    # Build prompt and call LLM
    prompt = build_prompt(text, args.context, args.mode, max_entities)
    messages = [{"role": "user", "content": prompt}]

    response_text, error = call_llm(messages, config)

    if error:
        print(f"[Nexus] LLM error: {error}", file=sys.stderr)
        print(json.dumps({"entities": [], "relations": [], "error": error}))
        sys.exit(1)

    print(f"[Nexus] LLM response: {len(response_text)} chars", file=sys.stderr)

    # Parse and filter
    data = extract_json_from_response(response_text)
    entities, relations = filter_and_deduplicate(
        data.get("entities", []),
        data.get("relations", []),
    )

    result = {
        "entities": entities,
        "relations": relations,
    }

    print(f"[Nexus] Extracted: {len(entities)} entities, {len(relations)} relations", file=sys.stderr)
    sys.stdout.write(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
