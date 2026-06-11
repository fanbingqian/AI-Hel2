"""
Nexus Edge Verification Script.
Judges whether inferred/synthesis edges are valid using LLM.

Input (stdin): JSON array of edge objects
  [{"from": "EntityA", "to": "EntityB", "relation": "related_to", "confidence": 0.35}, ...]

Output (stdout): JSON array of judgments
  [{"index": 0, "valid": true}, {"index": 1, "valid": false}, ...]

Config via environment variables (set by Rust caller):
  NEXUS_LLM_PROVIDER, NEXUS_LLM_MODEL, NEXUS_LLM_API_KEY, NEXUS_LLM_BASE_URL
"""

import sys
import os
import json

sys.stdout.reconfigure(encoding="utf-8")

MAX_EDGES_PER_BATCH = 10


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


def build_prompt(edges):
    """Build a prompt asking the LLM to judge if each relationship edge is valid."""
    lines = [
        "以下是知识图谱中的边（关系），判断每个关系是否合理。只回复 YES 或 NO，每行一个。",
        "",
    ]
    for i, edge in enumerate(edges):
        lines.append(
            f"{i+1}. {edge.get('from','')} --[{edge.get('relation','')}]--> {edge.get('to','')}"
        )
    lines.append("")
    lines.append("回复格式: 1. YES  2. NO  3. YES ...")
    return "\n".join(lines)


def call_llm(provider, model, api_key, base_url, prompt):
    """Call the LLM API and return the response text."""
    import urllib.request
    import urllib.error

    url = f"{base_url}/chat/completions"
    body = json.dumps({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "max_tokens": 50,
        "temperature": 0,
    }).encode("utf-8")

    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {api_key}",
    }

    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            return data["choices"][0]["message"]["content"].strip()
    except urllib.error.HTTPError as e:
        error_body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {error_body}")
    except Exception as e:
        raise RuntimeError(str(e))


def parse_judgments(response_text, count):
    """Parse LLM response like '1. YES  2. NO  3. YES' into boolean list."""
    results = []
    for i in range(count):
        results.append({"index": i, "valid": False})

    for line in response_text.split("\n"):
        line = line.strip()
        for i in range(1, count + 1):
            prefix = f"{i}."
            if line.startswith(prefix):
                rest = line[len(prefix):].strip().upper()
                if "YES" in rest:
                    results[i - 1]["valid"] = True
                break

    return results


def main():
    provider, model, api_key, base_url = get_llm_config()

    if not api_key and provider != "hermes_builtin":
        print(json.dumps({"error": "No API key configured for Nexus LLM"}))
        sys.exit(1)

    raw = sys.stdin.read()
    try:
        edges = json.loads(raw)
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"Invalid JSON input: {e}"}))
        sys.exit(1)

    if not isinstance(edges, list) or len(edges) == 0:
        print(json.dumps([]))
        return

    all_results = []

    for batch_start in range(0, len(edges), MAX_EDGES_PER_BATCH):
        batch = edges[batch_start:batch_start + MAX_EDGES_PER_BATCH]
        try:
            prompt = build_prompt(batch)
            response = call_llm(provider, model, api_key, base_url, prompt)
            results = parse_judgments(response, len(batch))
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
