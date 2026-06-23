"""Nexus Knowledge Engine Tools — HTTP bridge to Rust knowledge_cache.db.

Five tools that give the Agent autonomous access to the local knowledge graph.
Each tool calls the Rust Nexus HTTP server (127.0.0.1 on a dynamically-allocated
port, read from {hermes_home}/nexus_port).

Tools: nexus_map · nexus_search · nexus_detail · nexus_paths · nexus_neighbors
"""

from __future__ import annotations

import json
import logging
import urllib.request
import urllib.error
from pathlib import Path
from typing import Optional

from tools.registry import registry

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Port resolution — read the port file that Rust writes on startup
# ---------------------------------------------------------------------------

def _get_nexus_base() -> Optional[str]:
    """Return the Nexus HTTP base URL, or None if the server is not running."""
    hermes_home = Path.home() / ".hermes"
    port_file = hermes_home / "nexus_port"
    if not port_file.exists():
        # Also check AI-Hel2 data directory
        alt = Path.home() / ".ai-hel2" / "nexus_port"
        if alt.exists():
            port_file = alt
        else:
            logger.debug("Nexus port file not found at %s", port_file)
            return None
    try:
        port = port_file.read_text().strip()
        if not port:
            return None
        return f"http://127.0.0.1:{port}"
    except Exception:
        logger.debug("Failed to read nexus port file", exc_info=True)
        return None


def _http_get(path: str) -> str:
    """Make a GET request to the Nexus HTTP server. Returns JSON string."""
    base = _get_nexus_base()
    if not base:
        return json.dumps({"error": "Nexus knowledge engine is not running. Start AI-Hel2 first."})
    url = f"{base}{path}"
    try:
        req = urllib.request.Request(url, headers={"Accept": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as resp:
            body = resp.read().decode("utf-8")
            return body
    except urllib.error.URLError as e:
        return json.dumps({"error": f"Nexus connection failed: {e.reason}"})
    except Exception as e:
        return json.dumps({"error": f"Nexus request failed: {e}"})


def _check_nexus() -> bool:
    """Check whether the Nexus HTTP server is reachable."""
    base = _get_nexus_base()
    if not base:
        return False
    try:
        req = urllib.request.Request(f"{base}/nexus/map", headers={"Accept": "application/json"})
        with urllib.request.urlopen(req, timeout=3) as resp:
            return resp.status == 200
    except Exception:
        return False


# ---------------------------------------------------------------------------
# Tool schemas
# ---------------------------------------------------------------------------

NEXUS_MAP_SCHEMA = {
    "name": "nexus_map",
    "description": (
        "返回本地知识库的知识地图：领域分布、各领域关键实体、子领域结构、领域间关联桥接。"
        "Agent 据此鸟瞰知识库拓扑，判断覆盖范围，决定是否需要 nexus_search 深入搜索。"
    ),
    "parameters": {
        "type": "object",
        "properties": {},
        "required": [],
    },
}

NEXUS_SEARCH_SCHEMA = {
    "name": "nexus_search",
    "description": (
        "全文搜索本地知识库中的实体。返回匹配实体列表，含摘要信息。"
        "在 nexus_map 确认知识库有相关覆盖后使用，也可直接调用（跳过 nexus_map）。"
        "Agent 根据结果判断是否需要 nexus_detail 深入查看。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "q": {"type": "string", "description": "搜索关键词"},
            "namespace": {"type": "string", "description": "可选，限定命名空间过滤"},
            "limit": {"type": "integer", "description": "返回数量上限，默认 10"},
        },
        "required": ["q"],
    },
}

NEXUS_DETAIL_SCHEMA = {
    "name": "nexus_detail",
    "description": (
        "获取单个实体的完整信息，包括属性、入边关系（哪些实体指向它）、出边关系（它指向哪些实体）。"
        "用于 nexus_search 结果中感兴趣实体的深入查看。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "id": {"type": "string", "description": "实体 ID（nexus_search 返回结果中的 entity id）"},
        },
        "required": ["id"],
    },
}

NEXUS_PATHS_SCHEMA = {
    "name": "nexus_paths",
    "description": (
        "查找两个实体之间的最短关系路径（BFS，最多 4 跳）。"
        "用于理解概念之间的关联链条。from/to 参数同时接受实体名称（模糊匹配）和 UUID（精确匹配）。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "from": {"type": "string", "description": "起始实体名称或 UUID（名称做模糊匹配，UUID 精确匹配）"},
            "to": {"type": "string", "description": "目标实体名称或 UUID（名称做模糊匹配，UUID 精确匹配）"},
            "max_hops": {"type": "integer", "description": "最大跳数，默认 4"},
        },
        "required": ["from", "to"],
    },
}

NEXUS_NEIGHBORS_SCHEMA = {
    "name": "nexus_neighbors",
    "description": (
        "展开指定实体周边 N 跳的邻居网络（BFS）。"
        "用于浏览知识图谱中某个实体周围的相关概念。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "id": {"type": "string", "description": "实体 ID"},
            "hops": {"type": "integer", "description": "展开跳数，默认 2，最大 4"},
        },
        "required": ["id"],
    },
}


# ---------------------------------------------------------------------------
# Tool handlers
# ---------------------------------------------------------------------------

def nexus_map_handler(args: dict = None, **kwargs) -> str:
    """Handle nexus_map — return the full knowledge map."""
    return _http_get("/nexus/map")


def nexus_search_handler(args: dict, **kwargs) -> str:
    """Handle nexus_search — FTS5 search across entities."""
    q = args.get("q", "")
    if not q.strip():
        return json.dumps({"entities": [], "hint": "No query provided"})
    params = f"q={urllib.parse.quote(q)}"
    ns = args.get("namespace")
    if ns:
        params += f"&namespace={urllib.parse.quote(ns)}"
    limit = args.get("limit")
    if limit:
        params += f"&limit={int(limit)}"
    return _http_get(f"/nexus/search?{params}")


def nexus_detail_handler(args: dict, **kwargs) -> str:
    """Handle nexus_detail — get full entity detail."""
    eid = args.get("id", "")
    if not eid.strip():
        return json.dumps({"error": "id is required"})
    return _http_get(f"/nexus/entity/{urllib.parse.quote(eid)}")


def nexus_paths_handler(args: dict, **kwargs) -> str:
    """Handle nexus_paths — BFS shortest path between two entities."""
    from_id = args.get("from", "")
    to_id = args.get("to", "")
    if not from_id or not to_id:
        return json.dumps({"error": "from and to are required"})
    params = f"from={urllib.parse.quote(from_id)}&to={urllib.parse.quote(to_id)}"
    max_hops = args.get("max_hops", 4)
    params += f"&max_hops={int(max_hops)}"
    return _http_get(f"/nexus/paths?{params}")


def nexus_neighbors_handler(args: dict, **kwargs) -> str:
    """Handle nexus_neighbors — BFS neighbor expansion."""
    eid = args.get("id", "")
    if not eid.strip():
        return json.dumps({"error": "id is required"})
    hops = args.get("hops", 2)
    return _http_get(f"/nexus/neighbors/{urllib.parse.quote(eid)}?hops={int(hops)}")


# ---------------------------------------------------------------------------
# Register all five tools
# ---------------------------------------------------------------------------

registry.register(
    name="nexus_map",
    toolset="nexus",
    schema=NEXUS_MAP_SCHEMA,
    handler=nexus_map_handler,
    emoji="🗺️",
)

registry.register(
    name="nexus_search",
    toolset="nexus",
    schema=NEXUS_SEARCH_SCHEMA,
    handler=nexus_search_handler,
    emoji="🔍",
)

registry.register(
    name="nexus_detail",
    toolset="nexus",
    schema=NEXUS_DETAIL_SCHEMA,
    handler=nexus_detail_handler,
    emoji="📋",
)

registry.register(
    name="nexus_paths",
    toolset="nexus",
    schema=NEXUS_PATHS_SCHEMA,
    handler=nexus_paths_handler,
    emoji="🔗",
)

registry.register(
    name="nexus_neighbors",
    toolset="nexus",
    schema=NEXUS_NEIGHBORS_SCHEMA,
    handler=nexus_neighbors_handler,
    emoji="🌐",
)

# ---------------------------------------------------------------------------
# CRUD tools: create / update / delete entities in the knowledge graph
# ---------------------------------------------------------------------------

NEXUS_CREATE_SCHEMA = {
    "name": "nexus_create",
    "description": (
        "在本地知识库中创建一个新的 Markdown 文档。传入文件名和完整内容。"
        "文件将写入 wiki 目录，后续 Nexus 会自动提取实体和关系写入图谱。"
        "适合写入报告、文章、笔记等结构化内容。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "filename": {"type": "string", "description": "文件名，如 '2024年度总结报告.md' 或 'AI行业分析.md'"},
            "content": {"type": "string", "description": "Markdown 格式的完整文档内容"},
        },
        "required": ["filename", "content"],
    },
}

NEXUS_UPDATE_SCHEMA = {
    "name": "nexus_update",
    "description": (
        "更新本地知识库中已有文档的内容。传入文件路径和新的完整内容。"
        "Nexus 会自动重新提取实体和关系，更新图谱中的对应数据。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "文档路径，如 '2024年度总结报告.md'"},
            "content": {"type": "string", "description": "更新后的完整 Markdown 内容"},
        },
        "required": ["path", "content"],
    },
}

NEXUS_DELETE_SCHEMA = {
    "name": "nexus_delete",
    "description": (
        "删除本地知识库中的文档。传入文件路径，文件将被永久删除。"
        "Nexus 会自动清理该文档关联的实体和关系。"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "要删除的文档路径，如 '过期的草稿.md'"},
        },
        "required": ["path"],
    },
}


def nexus_create_handler(args: dict, **kwargs) -> str:
    """Create a wiki document via HTTP POST."""
    import json as _json
    filename = args.get("filename", "")
    content = args.get("content", "")
    if not filename.strip():
        return _json.dumps({"error": "filename is required"})
    body = _json.dumps({"filename": filename, "content": content})
    return _http_post("/nexus/wiki", body)


def nexus_update_handler(args: dict, **kwargs) -> str:
    """Update a wiki document via HTTP PUT."""
    import json as _json
    path = args.get("path", "")
    content = args.get("content", "")
    if not path.strip():
        return _json.dumps({"error": "path is required"})
    body = _json.dumps({"content": content})
    return _http_put(f"/nexus/wiki/{urllib.parse.quote(path)}", body)


def nexus_delete_handler(args: dict, **kwargs) -> str:
    """Delete a wiki document via HTTP DELETE."""
    import json as _json
    path = args.get("path", "")
    if not path.strip():
        return _json.dumps({"error": "path is required"})
    return _http_delete(f"/nexus/wiki/delete/{urllib.parse.quote(path)}")


def _http_post(path: str, body: str) -> str:
    """Make a POST request to the Nexus HTTP server."""
    base = _get_nexus_base()
    if not base:
        return json.dumps({"error": "Nexus knowledge engine is not running."})
    url = f"{base}{path}"
    try:
        data = body.encode("utf-8")
        req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json", "Accept": "application/json"}, method="POST")
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.read().decode("utf-8")
    except urllib.error.URLError as e:
        return json.dumps({"error": f"Nexus connection failed: {e.reason}"})
    except Exception as e:
        return json.dumps({"error": f"Nexus request failed: {e}"})


def _http_put(path: str, body: str) -> str:
    """Make a PUT request to the Nexus HTTP server."""
    base = _get_nexus_base()
    if not base:
        return json.dumps({"error": "Nexus knowledge engine is not running."})
    url = f"{base}{path}"
    try:
        data = body.encode("utf-8")
        req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json", "Accept": "application/json"}, method="PUT")
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.read().decode("utf-8")
    except urllib.error.URLError as e:
        return json.dumps({"error": f"Nexus connection failed: {e.reason}"})
    except Exception as e:
        return json.dumps({"error": f"Nexus request failed: {e}"})


def _http_delete(path: str) -> str:
    """Make a DELETE request to the Nexus HTTP server."""
    base = _get_nexus_base()
    if not base:
        return json.dumps({"error": "Nexus knowledge engine is not running."})
    url = f"{base}{path}"
    try:
        req = urllib.request.Request(url, headers={"Accept": "application/json"}, method="DELETE")
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.read().decode("utf-8")
    except urllib.error.URLError as e:
        return json.dumps({"error": f"Nexus connection failed: {e.reason}"})
    except Exception as e:
        return json.dumps({"error": f"Nexus request failed: {e}"})


# Register CRUD tools
registry.register(
    name="nexus_create",
    toolset="nexus",
    schema=NEXUS_CREATE_SCHEMA,
    handler=nexus_create_handler,
    emoji="➕",
)

registry.register(
    name="nexus_update",
    toolset="nexus",
    schema=NEXUS_UPDATE_SCHEMA,
    handler=nexus_update_handler,
    emoji="✏️",
)

registry.register(
    name="nexus_delete",
    toolset="nexus",
    schema=NEXUS_DELETE_SCHEMA,
    handler=nexus_delete_handler,
    emoji="🗑️",
)
