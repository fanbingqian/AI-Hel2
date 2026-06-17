"""AI-Hel Knowledge Base Memory Provider.

Integrates AI-Hel's local knowledge graph (SQLite + Wiki files) with
the Hermes Agent via the standard MemoryProvider interface.

Three-segment prefetch on every turn:
1. Query-matching entities (FTS5 search)
2. Matching wiki file names
3. Recent changes since session start (query-independent)
"""

from __future__ import annotations

import json
import logging
import os
import re
import threading
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

from agent.memory_provider import MemoryProvider

from .knowledge_store import KnowledgeStore
from .tools import (
    ALL_TOOL_SCHEMAS,
    handle_get_entity,
    handle_list_wiki,
    handle_read_wiki,
    handle_recent,
    handle_save,
    handle_search,
)

logger = logging.getLogger(__name__)


class AiHelMemoryProvider(MemoryProvider):
    """MemoryProvider backed by AI-Hel's local knowledge graph."""

    def __init__(self) -> None:
        self._store: Optional[KnowledgeStore] = None
        self._wiki_dir: Optional[Path] = None
        self._prefetch_cache: Optional[str] = None
        self._lock = threading.Lock()
        self._session_start = ""
        self._session_id = ""

    # ── Identification ───────────────────────────────────────────

    @property
    def name(self) -> str:
        return "aihel"

    def is_available(self) -> bool:
        return True

    # ── Lifecycle ────────────────────────────────────────────────

    def initialize(self, session_id: str, **kwargs) -> None:
        # Use AI_HEL2_HOME env var (set by Rust launcher), fallback to ~/.ai-hel2
        hel2_home = os.environ.get("AI_HEL2_HOME", os.path.expanduser("~/.ai-hel2"))
        self._db_path = Path(hel2_home) / "knowledge_cache.db"
        self._wiki_dir = Path(hel2_home) / "wiki"
        self._store = KnowledgeStore(self._db_path)
        self._session_start = datetime.now(timezone.utc).isoformat()
        self._session_id = session_id

        count = self._store.get_entity_count()
        stats = self._store.get_stats()
        logger.info(
            "AI-Hel knowledge store initialized: %s entities, %s relations, wiki=%s",
            count,
            stats.get("total_relations", 0),
            self._wiki_dir,
        )

    def shutdown(self) -> None:
        self._store = None
        self._wiki_dir = None

    # ── System prompt ────────────────────────────────────────────

    def system_prompt_block(self) -> str:
        if not self._store:
            return ""
        snapshot = self._store.get_knowledge_snapshot()
        count = self._store.get_entity_count()
        return (
            f"## AI-Hel 知识库\n"
            f"当前 {count} 个实体\n\n"
            f"{snapshot}\n\n"
            f"可用工具: aihel_search(搜索知识图谱实体) "
            f"aihel_get_entity(查看实体详情与关系) "
            f"aihel_recent(查看最近更新) "
            f"aihel_save(保存知识) "
            f"aihel_list_wiki(浏览Wiki文档) "
            f"aihel_read_wiki(读取Wiki文档全文)"
        )

    # ── Per-turn prefetch ────────────────────────────────────────

    def prefetch(self, query: str, *, session_id: str = "") -> str:
        with self._lock:
            result = self._prefetch_cache
            self._prefetch_cache = None
        if result:
            return f"<memory-context>\n{result}\n</memory-context>"
        return ""

    def queue_prefetch(self, query: str, *, session_id: str = "") -> None:
        if not self._store or not query:
            return

        # Segment 1: Query-matching entities (FTS5)
        matched = self._store.search_entities(query, None, 8)

        # Segment 2: Matching wiki files
        matching_files = self._search_wiki_files(query, 3)

        # Segment 3: Recent changes since session start (query-independent)
        recent = self._store.get_recent_changes(self._session_start, 5)

        # Deduplicate recent from matched
        recent_ids = {r["id"] for r in recent}
        matched = [m for m in matched if m["id"] not in recent_ids]

        parts: List[str] = []
        if matched:
            parts.append("## 匹配当前话题的实体\n" + self._format_entities(matched))
        if matching_files:
            parts.append(
                "## 匹配的文档\n"
                + self._format_files(matching_files)
                + "\n(使用 aihel_read_wiki <路径> 读取文档全文)"
            )
        if recent:
            parts.append("## 最近新增/更新\n" + self._format_recent(recent))

        with self._lock:
            self._prefetch_cache = "\n\n".join(parts) if parts else None

    # ── Turn sync ────────────────────────────────────────────────

    def sync_turn(
        self, user_content: str, assistant_content: str, *, session_id: str = ""
    ) -> None:
        """Extract [[wikilinks]], **bold**, 《book》 from conversation.

        Writes to _auto/chat_*.md for FileWatcher pick-up and entity extraction."""
        if not self._wiki_dir:
            return

        combined = f"{user_content} {assistant_content}"
        entities: List[str] = []
        entities.extend(re.findall(r"\[\[([^\]]+)\]\]", combined))
        entities.extend(re.findall(r"\*\*([^*]+)\*\*", combined))
        entities.extend(re.findall(r"《([^》]+)》", combined))

        if not entities:
            return

        seen: set = set()
        unique = []
        for e in entities:
            e = e.strip()
            if e and e not in seen and len(e) < 80:
                seen.add(e)
                unique.append(e)

        if not unique:
            return

        auto_dir = self._wiki_dir / "_auto"
        try:
            auto_dir.mkdir(parents=True, exist_ok=True)
        except OSError:
            return

        timestamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
        filename = f"chat_{timestamp}.md"

        lines = [
            "---",
            f"title: 对话提取 - {timestamp}",
            "tags: [auto-extracted]",
            "---",
            "",
            "## 用户消息",
            user_content[:500],
            "",
            "## 助手回复",
            assistant_content[:500],
            "",
            "## 提取的实体",
        ]
        for e in unique:
            lines.append(f"- **{e}**")

        try:
            (auto_dir / filename).write_text("\n".join(lines), encoding="utf-8")
        except OSError:
            pass

    # ── Tools ────────────────────────────────────────────────────

    def get_tool_schemas(self) -> List[Dict[str, Any]]:
        return list(ALL_TOOL_SCHEMAS)

    def handle_tool_call(
        self, tool_name: str, args: Dict[str, Any], **kwargs
    ) -> str:
        if not self._store:
            return json.dumps({"error": "Knowledge store not initialized"})

        if tool_name == "aihel_search":
            return handle_search(self._store, args)
        if tool_name == "aihel_get_entity":
            return handle_get_entity(self._store, args)
        if tool_name == "aihel_recent":
            return handle_recent(self._store, self._session_start, args)
        if tool_name == "aihel_list_wiki":
            return handle_list_wiki(self._wiki_dir, args)
        if tool_name == "aihel_read_wiki":
            return handle_read_wiki(self._wiki_dir, args)
        if tool_name == "aihel_save":
            return handle_save(self._wiki_dir, args)

        return json.dumps({"error": f"Unknown tool: {tool_name}"})

    # ── Private helpers ──────────────────────────────────────────

    def _search_wiki_files(self, query: str, limit: int) -> List[Dict]:
        if not self._wiki_dir or not self._wiki_dir.exists():
            return []

        results: List[Dict] = []
        query_lower = query.lower()
        for p in sorted(self._wiki_dir.rglob("*.md")):
            if "_auto" in p.parts:
                continue
            if query_lower in p.stem.lower():
                try:
                    rel = str(p.relative_to(self._wiki_dir)).replace("\\", "/")
                    results.append({"path": rel, "name": p.stem})
                except Exception:
                    continue
            if len(results) >= limit:
                break
        return results

    @staticmethod
    def _format_entities(entities: List[Dict]) -> str:
        lines: List[str] = []
        for e in entities:
            desc = (e.get("description") or "")[:80]
            etype = e.get("entity_type", "?")
            lines.append(f"- **{e['name']}** ({etype}): {desc}")
        return "\n".join(lines)

    @staticmethod
    def _format_files(files: List[Dict]) -> str:
        lines: List[str] = []
        for f in files:
            lines.append(f"- `{f['path']}` ({f['name']})")
        return "\n".join(lines)

    @staticmethod
    def _format_recent(entities: List[Dict]) -> str:
        lines: List[str] = []
        for e in entities:
            updated = e.get("updated_at", "")[:16]
            lines.append(
                f"- **{e['name']}** ({e.get('entity_type', '?')}): {updated}"
            )
        return "\n".join(lines)


def register(ctx):
    """Register the AI-Hel memory provider with the Agent plugin context."""
    ctx.register_memory_provider(AiHelMemoryProvider())
