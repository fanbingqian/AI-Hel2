"""Tool schemas (OpenAI function-calling format) and handler functions.

Each handler receives the necessary state and returns a JSON string."""

from __future__ import annotations

import json
import re
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List

from .knowledge_store import KnowledgeStore

# ── Tool schemas ────────────────────────────────────────────────

AIHEL_SEARCH_SCHEMA: Dict[str, Any] = {
    "name": "aihel_search",
    "description": (
        "Search the local AI-Hel knowledge graph for entities (concepts, people, "
        "projects, events, etc.) matching a keyword or phrase. "
        "Returns entity name, type, description, and confidence. "
        "Use when you need to recall stored context about a topic before answering."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search keywords or phrase.",
            },
            "entity_type": {
                "type": "string",
                "enum": ["concept", "content", "person", "event", "artifact"],
                "description": "Filter by entity type (optional).",
            },
            "limit": {
                "type": "integer",
                "description": "Max results (default: 10, max: 30).",
            },
        },
        "required": ["query"],
    },
}

AIHEL_GET_ENTITY_SCHEMA: Dict[str, Any] = {
    "name": "aihel_get_entity",
    "description": (
        "Get full details for a specific knowledge entity by ID, including its "
        "inbound and outbound relationships to other entities. "
        "Use after aihel_search to explore an entity's connections."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "entity_id": {
                "type": "string",
                "description": "The entity ID from aihel_search results.",
            },
        },
        "required": ["entity_id"],
    },
}

AIHEL_RECENT_SCHEMA: Dict[str, Any] = {
    "name": "aihel_recent",
    "description": (
        "List recently added or updated entities in the knowledge base. "
        "Use to discover what new knowledge is available without guessing keywords. "
        "Also use when the user asks 'what's new' or 'any updates?'"
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "limit": {
                "type": "integer",
                "description": "Max results (default: 10, max: 30).",
            },
        },
        "required": [],
    },
}

AIHEL_LIST_WIKI_SCHEMA: Dict[str, Any] = {
    "name": "aihel_list_wiki",
    "description": (
        "Browse or search the knowledge base wiki file directory. "
        "Returns file paths, titles, tags, and modification times. "
        "Use to discover what documents are available. "
        "To read a document's full content, use aihel_read_wiki with the file path."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Directory path to browse, e.g. 'projects/'. Empty = root.",
            },
            "query": {
                "type": "string",
                "description": "Filename keyword search. Overrides path when provided.",
            },
        },
        "required": [],
    },
}

AIHEL_READ_WIKI_SCHEMA: Dict[str, Any] = {
    "name": "aihel_read_wiki",
    "description": (
        "Read the full Markdown content of a wiki document. "
        "Use when aihel_search entity summaries are not detailed enough, "
        "or when the prefetch hints that a matching document exists."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": (
                    "Relative file path from aihel_list_wiki results, "
                    "e.g. 'projects/MyProject.md'"
                ),
            },
        },
        "required": ["file_path"],
    },
}

AIHEL_SAVE_SCHEMA: Dict[str, Any] = {
    "name": "aihel_save",
    "description": (
        "Save a durable fact or discovery to the AI-Hel knowledge base. "
        "This writes a Markdown wiki file which will be automatically parsed "
        "for entities ([[wikilinks]], **bold terms**, 《book titles》). "
        "Use for important facts, user preferences, decisions, or project context "
        "that should persist across sessions. Do NOT save temporary task state."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "description": "Title for the knowledge entry (becomes the wiki filename).",
            },
            "content": {
                "type": "string",
                "description": "Markdown content to save. Use [[wikilinks]] for entities.",
            },
            "tags": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Tags for categorization (optional).",
            },
            "namespace": {
                "type": "string",
                "description": "Subfolder within wiki/ (optional, default: 'general').",
            },
        },
        "required": ["title", "content"],
    },
}

ALL_TOOL_SCHEMAS = [
    AIHEL_SEARCH_SCHEMA,
    AIHEL_GET_ENTITY_SCHEMA,
    AIHEL_RECENT_SCHEMA,
    AIHEL_LIST_WIKI_SCHEMA,
    AIHEL_READ_WIKI_SCHEMA,
    AIHEL_SAVE_SCHEMA,
]

# ── Helper ──────────────────────────────────────────────────────

def _parse_frontmatter(content: str) -> Dict[str, Any]:
    """Parse YAML-like frontmatter from markdown content.

    Simple parser that doesn't require PyYAML dependency."""
    if not content.startswith("---"):
        return {}
    end = content.find("---", 3)
    if end == -1:
        return {}
    fm: Dict[str, Any] = {}
    for line in content[3:end].strip().split("\n"):
        line = line.strip()
        if ":" in line:
            key, _, val = line.partition(":")
            key = key.strip()
            val = val.strip().strip("\"'")
            if val.startswith("[") and val.endswith("]"):
                val = [v.strip().strip("\"'") for v in val[1:-1].split(",") if v.strip()]
            fm[key] = val
    return fm

# ── Handlers ────────────────────────────────────────────────────

def handle_search(store: KnowledgeStore, args: dict) -> str:
    query = args.get("query", "")
    entity_type = args.get("entity_type")
    limit = min(args.get("limit", 10), 30)
    results = store.search_entities(query, entity_type, limit)
    return json.dumps({"results": results, "count": len(results)}, ensure_ascii=False)


def handle_get_entity(store: KnowledgeStore, args: dict) -> str:
    entity_id = args.get("entity_id", "")
    entity = store.get_entity(entity_id)
    if not entity:
        return json.dumps({"error": f"Entity not found: {entity_id}"})
    inbound, outbound = store.get_entity_relations(entity_id)
    return json.dumps(
        {
            "entity": entity,
            "inbound_relations": inbound,
            "outbound_relations": outbound,
        },
        ensure_ascii=False,
    )


def handle_recent(store: KnowledgeStore, session_start: str, args: dict) -> str:
    limit = min(args.get("limit", 10), 30)
    results = store.get_recent_changes(session_start, limit)
    return json.dumps({"results": results, "count": len(results)}, ensure_ascii=False)


def handle_list_wiki(wiki_dir: Path, args: dict) -> str:
    query = args.get("query", "")
    subpath = args.get("path", "")

    # Security: ensure we don't escape wiki_dir
    search_root = (wiki_dir / subpath).resolve() if subpath else wiki_dir.resolve()
    if not str(search_root).startswith(str(wiki_dir.resolve())):
        return json.dumps({"error": "Path traversal denied"})

    if not search_root.exists():
        return json.dumps({"files": [], "count": 0})

    files: List[Dict] = []
    for p in sorted(search_root.rglob("*.md")):
        try:
            rel = str(p.relative_to(wiki_dir)).replace("\\", "/")
            if query and query.lower() not in p.stem.lower():
                continue
            stat = p.stat()
            content = p.read_text(encoding="utf-8", errors="replace")
            fm = _parse_frontmatter(content)
            files.append(
                {
                    "path": rel,
                    "name": p.stem,
                    "title": fm.get("title", p.stem),
                    "tags": fm.get("tags", []),
                    "size": stat.st_size,
                    "modified_at": datetime.fromtimestamp(
                        stat.st_mtime, tz=timezone.utc
                    ).isoformat(),
                }
            )
        except Exception:
            continue

    return json.dumps({"files": files, "count": len(files)}, ensure_ascii=False)


def handle_read_wiki(wiki_dir: Path, args: dict) -> str:
    file_path = args.get("file_path", "")

    # Security: prevent path traversal
    resolved = (wiki_dir / file_path).resolve()
    if not str(resolved).startswith(str(wiki_dir.resolve())):
        return json.dumps({"error": "Path traversal denied"})

    if not resolved.exists():
        return json.dumps({"error": f"File not found: {file_path}"})
    if not resolved.suffix == ".md":
        return json.dumps({"error": "Only .md files can be read"})

    try:
        content = resolved.read_text(encoding="utf-8")
        if len(content) > 4000:
            content = content[:4000] + "\n\n...(truncated, use smaller sections if needed)"
        return json.dumps(
            {"file_path": file_path, "content": content}, ensure_ascii=False
        )
    except Exception as e:
        return json.dumps({"error": str(e)})


def handle_save(wiki_dir: Path, args: dict) -> str:
    title = args.get("title", "untitled")
    content = args.get("content", "")
    tags = args.get("tags", [])
    namespace = args.get("namespace", "general")

    # Sanitize filename
    safe_name = re.sub(r'[/\\:*?"<>|]', "_", title)
    if not safe_name.endswith(".md"):
        safe_name += ".md"

    # Determine target directory
    target_dir = wiki_dir / namespace
    target_dir.mkdir(parents=True, exist_ok=True)

    # Build frontmatter
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    tags_yaml = json.dumps(tags, ensure_ascii=False) if tags else "[]"
    fm = f"---\ntitle: {title}\ntags: {tags_yaml}\ncreated: {now}\n---\n\n"

    # Write
    target_path = target_dir / safe_name
    target_path.write_text(fm + content, encoding="utf-8")

    return json.dumps(
        {
            "status": "saved",
            "file_path": f"{namespace}/{safe_name}",
            "title": title,
        },
        ensure_ascii=False,
    )
