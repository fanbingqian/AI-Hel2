"""
Nexus Transitive Inference Script.
Judges whether A→B→C paths imply an A→C relationship using LLM.

Input (stdin): JSON array of path candidates
  [{
    "from": {"name": "A", "type": "concept", "desc": "..."},
    "via":  {"name": "B", "type": "concept", "desc": "..."},
    "to":   {"name": "C", "type": "concept", "desc": "..."},
    "rel1": "depends_on",   // A→B relation type
    "rel2": "contains",     // B→C relation type
    "rel1_weight": 0.8,
    "rel2_weight": 0.7,
    "to_exists": true       // C already exists in the graph?
  }, ...]

Output (stdout): JSON array of judgments
  [{
    "index": 0,
    "valid": true,
    "relation_type": "depends_on",
    "confidence": 0.45,
    "confidence_reason": "...",
    "new_entity": null
  }, ...]

Config via environment variables:
  NEXUS_LLM_PROVIDER, NEXUS_LLM_MODEL, NEXUS_LLM_API_KEY, NEXUS_LLM_BASE_URL
"""

import sys
import os
import json

sys.stdout.reconfigure(encoding="utf-8")

MAX_PATHS_PER_BATCH = 8


def get_llm_config():
    provider = os.environ.get("NEXUS_LLM_PROVIDER", "anthropic")
    model = os.environ.get("NEXUS_LLM_MODEL", "claude-sonnet-4-6")
    api_key = os.environ.get("NEXUS_LLM_API_KEY", "")
    base_url = os.environ.get("NEXUS_LLM_BASE_URL", "")

    if not base_url:
        provider_urls = {
            "anthropic": "https://api.anthropic.com",
            "openai": "https://api.openai.com",
            "deepseek": "https://api.deepseek.com",
            "hermes_builtin": "http://127.0.0.1:18642",
        }
        base_url = provider_urls.get(provider, "https://api.anthropic.com")
    if base_url and not base_url.rstrip("/").endswith("/v1"):
        base_url = base_url.rstrip("/") + "/v1"

    return provider, model, api_key, base_url


def build_prompt(paths):
    """Build a prompt asking the LLM to judge transitive paths."""
    lines = [
        "## 角色",
        "你是 Nexus 知识引擎的推理审核员。你的任务是判断传递关系链是否成立，并在必要时创建新的推断实体。",
        "",
        "## 规则",
        "1. 已知 A→B (rel1) 和 B→C (rel2)，判断 A→C 之间是否存在合理的语义关系",
        "2. 如果可以推断 A→C 的关系，给出：关系类型 + 置信度 + 理由",
        "3. 关系类型从以下选择: depends_on, uses, contains, related_to, part_of, causes, creates, located_in",
        "4. 置信度分级:",
        "   0.4-0.5 — 强传递链(如 is_a→is_a, part_of→part_of)",
        "   0.2-0.4 — 中传递链(如 depends_on→uses)",
        "   0.1-0.2 — 弱关联链(如 related_to→related_to)",
        "   0    — 无法推断(valid=false)",
        "5. 不确定时倾向于返回 valid=false（宁可少推断）",
        "6. 如果 C 是已有实体(to_exists=true)，只判断关系，不创建新实体",
        "7. 如果 C 需要作为一个新的概念实体(to_exists=false)，同时输出 new_entity 字段",
        "   new_entity 包含: name(推断出的实体名), entity_type, description, confidence, confidence_reason",
        "   entity_type 可是: concept/project/tool/location/organization/person/time",
        "",
    ]

    for i, p in enumerate(paths):
        lines.append(
            f"候选 {i+1}: "
            f"[{p['from']['name']}]({p['from']['type']}) --{p['rel1']}({p['rel1_weight']})--> "
            f"[{p['via']['name']}]({p['via']['type']}) --{p['rel2']}({p['rel2_weight']})--> "
            f"[{p['to']['name']}]({p['to']['type']})"
        )
        if p['from'].get('desc'):
            lines.append(f"  {p['from']['name']} 描述: {p['from']['desc'][:150]}")
        if p['to'].get('desc'):
            lines.append(f"  {p['to']['name']} 描述: {p['to']['desc'][:150]}")

    lines.append("")
    lines.append("## 输出格式")
    lines.append("每行一个 JSON，只输出 JSON 数组，不要其他文字。已有实体(to_exists=true)格式:")
    lines.append("""[{"index": 0, "valid": true, "relation_type": "uses", "confidence": 0.3, "confidence_reason": "A通过B间接使用C"}]""")
    lines.append("需要创建新实体(to_exists=false 且 valid=true)格式:")
    lines.append("""[{"index": 0, "valid": true, "relation_type": "depends_on", "confidence": 0.25, "confidence_reason": "...", "new_entity": {"name": "实体名", "entity_type": "concept", "description": "通过传递推理从A→B→C推断", "confidence": 0.25, "confidence_reason": "弱传递链推断"}}]""")

    return "\n".join(lines)


def call_llm(provider, model, api_key, base_url, prompt):
    import urllib.request
    import urllib.error

    url = f"{base_url}/chat/completions"
    body = json.dumps({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 500,
        "temperature": 0.1,
    }).encode("utf-8")

    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {api_key}",
    }

    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=45) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            return data["choices"][0]["message"]["content"].strip()
    except urllib.error.HTTPError as e:
        error_body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {error_body[:300]}")
    except Exception as e:
        raise RuntimeError(str(e))


def parse_response(text, count):
    """Parse LLM response JSON array."""
    results = [{"index": i, "valid": False} for i in range(count)]

    text = text.strip()
    # Extract JSON array from markdown fences
    if "```" in text:
        start = text.find("[")
        end = text.rfind("]")
        if start >= 0 and end > start:
            text = text[start:end + 1]

    try:
        parsed = json.loads(text)
        if isinstance(parsed, list):
            for item in parsed:
                idx = item.get("index", 0)
                if 0 <= idx < count:
                    results[idx] = {
                        "index": idx,
                        "valid": item.get("valid", False),
                        "relation_type": item.get("relation_type", "related_to"),
                        "confidence": min(max(item.get("confidence", 0.3), 0.05), 0.5),
                        "confidence_reason": item.get("confidence_reason", ""),
                    }
    except json.JSONDecodeError:
        # Fallback: try parsing line by line
        for line in text.split("\n"):
            line = line.strip()
            try:
                item = json.loads(line)
                idx = item.get("index", 0)
                if 0 <= idx < count:
                    results[idx] = {
                        "index": idx,
                        "valid": item.get("valid", False),
                        "relation_type": item.get("relation_type", "related_to"),
                        "confidence": min(max(item.get("confidence", 0.3), 0.05), 0.5),
                        "confidence_reason": item.get("confidence_reason", ""),
                    }
            except json.JSONDecodeError:
                continue

    return results


def main():
    provider, model, api_key, base_url = get_llm_config()

    if not api_key and provider != "hermes_builtin":
        print(json.dumps({"error": "No API key configured for Nexus LLM"}))
        sys.exit(1)

    raw = sys.stdin.read()
    try:
        paths = json.loads(raw)
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"Invalid JSON input: {e}"}))
        sys.exit(1)

    if not isinstance(paths, list) or len(paths) == 0:
        print(json.dumps([]))
        return

    all_results = []

    for batch_start in range(0, len(paths), MAX_PATHS_PER_BATCH):
        batch = paths[batch_start:batch_start + MAX_PATHS_PER_BATCH]
        try:
            prompt = build_prompt(batch)
            response = call_llm(provider, model, api_key, base_url, prompt)
            results = parse_response(response, len(batch))
            for r in results:
                r["index"] = batch_start + r["index"]
            all_results.extend(results)
        except Exception as e:
            for j in range(len(batch)):
                all_results.append({
                    "index": batch_start + j,
                    "valid": False,
                    "error": str(e),
                })

    print(json.dumps(all_results))


if __name__ == "__main__":
    main()
