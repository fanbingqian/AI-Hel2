"""Knowledge base overview cache with debounced rebuild (V2.3).

Generates lightweight text summaries of the knowledge graph for injection
into the LLM system prompt. Uses file-based caching under ~/.heimdall/cache/.

Two cache layers:
  - overview_{namespace}.txt — per-agent domain overview
  - overview_all.txt — cross-domain global overview

Rebuild is debounced (2s window) so rapid writes only trigger one regeneration.
"""

import json
import logging
import threading
from pathlib import Path

logger = logging.getLogger(__name__)

HEIMDALL_HOME = Path.home() / ".heimdall"
CACHE_DIR = HEIMDALL_HOME / "cache"

DEBOUNCE_SEC = 2
ALL_REBUILD_DELTA = 0.10


# ---------------------------------------------------------------------------
# Community cache — written by community.py, read by OverviewCache
# ---------------------------------------------------------------------------

class CommunityCache:
    """File-based cache for community detection results."""

    def __init__(self):
        CACHE_DIR.mkdir(parents=True, exist_ok=True)

    def read(self, namespace: str) -> list[dict]:
        path = CACHE_DIR / f"community_{namespace}.json"
        if not path.exists():
            return []
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            return []

    def write(self, namespace: str, communities: list[dict]):
        path = CACHE_DIR / f"community_{namespace}.json"
        path.write_text(
            json.dumps(communities, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )


# ---------------------------------------------------------------------------
# Overview cache
# ---------------------------------------------------------------------------

def _group_by_type(entities: list[dict]) -> dict[str, list[dict]]:
    """Group entities by their primary type."""
    groups: dict[str, list[dict]] = {}
    for e in entities:
        types = e.get("types", ["concept"])
        primary = types[0] if types else "concept"
        groups.setdefault(primary, []).append(e)
    return groups


def behavior_guidance(entity_count: int) -> str:
    """Return scale-adaptive retrieval guidance for the system prompt."""
    base = "知识库可能包含比训练数据更精准的领域知识。"
    if entity_count < 1000:
        return base + "当前知识库较小，若相关请查 search_knowledge。"
    elif entity_count < 10000:
        return base + "引用具体概念或用户之前的分析时，优先查 search_knowledge。"
    else:
        return (
            base + "知识库已积累大量领域知识，你的训练数据可能不够深入。"
            "优先使用 search_knowledge 获取精准信息。"
        )


class OverviewCache:
    """Generates and caches knowledge overviews with debounced rebuild."""

    def __init__(self, store, community_cache: CommunityCache):
        self._store = store
        self._community_cache = community_cache
        self._pending: dict[str, threading.Timer] = {}
        self._last_all_count: int = 0
        CACHE_DIR.mkdir(parents=True, exist_ok=True)

        # Ensure caches exist on startup
        self._ensure_startup_caches()

    def _ensure_startup_caches(self):
        if not (CACHE_DIR / "overview_all.txt").exists():
            self._rebuild_all()
        try:
            for ns_info in self._store.list_namespaces():
                ns = ns_info.get("namespace", "general")
                if not (CACHE_DIR / f"overview_{ns}.txt").exists():
                    self.rebuild(ns)
        except Exception:
            pass

    # -------------------------------------------------------------------
    # Public API
    # -------------------------------------------------------------------

    def schedule_rebuild(self, namespace: str):
        """Call after writes. Debounces rebuild by DEBOUNCE_SEC seconds."""
        if namespace in self._pending:
            self._pending[namespace].cancel()
        timer = threading.Timer(DEBOUNCE_SEC, self._do_rebuild, args=[namespace])
        timer.daemon = True
        self._pending[namespace] = timer
        timer.start()

    def rebuild(self, namespace: str):
        """Immediately rebuild the overview for a namespace."""
        try:
            entities = self._store.list_by_namespace(namespace)
        except Exception:
            entities = []

        if len(entities) < 50:
            groups = _group_by_type(entities)
        else:
            communities = self._community_cache.read(namespace)
            if communities:
                groups = {
                    c.get("name", f"community_{i}"): c.get("entities", [])
                    for i, c in enumerate(communities)
                }
            else:
                groups = _group_by_type(entities)

        summary_lines = []
        for group_name, group_entities in groups.items():
            count = len(group_entities)
            top_names = [e.get("name", "?") for e in group_entities[:3]]
            summary_lines.append(
                f"{group_name}({count}) — 核心: {'/'.join(top_names)}"
            )

        overview_text = (
            f"{namespace}({len(entities)}实体, {len(groups)}个子领域):\n"
            + "\n".join(f"  {line}" for line in summary_lines)
        )
        self._write_file(f"overview_{namespace}.txt", overview_text)

    def read(self, namespace: str | None) -> str:
        """Read cached overview. namespace=None reads overview_all.txt."""
        if namespace:
            return self._read_file(f"overview_{namespace}.txt")
        return self._read_file("overview_all.txt")

    # -------------------------------------------------------------------
    # Internal
    # -------------------------------------------------------------------

    def _do_rebuild(self, namespace: str):
        self._pending.pop(namespace, None)
        self.rebuild(namespace)
        self._maybe_rebuild_all()

    def _maybe_rebuild_all(self):
        try:
            total = self._store.count_all()
        except Exception:
            return
        threshold = max(int(total * ALL_REBUILD_DELTA), 50)
        if abs(total - self._last_all_count) > threshold:
            self._rebuild_all()
            self._last_all_count = total

    def _rebuild_all(self):
        try:
            entities = self._store.list_all()
        except Exception:
            entities = []

        groups = _group_by_type(entities)
        summary_lines = []
        for group_name, group_entities in groups.items():
            count = len(group_entities)
            top_names = [e.get("name", "?") for e in group_entities[:3]]
            summary_lines.append(
                f"{group_name}({count}) — 核心: {'/'.join(top_names)}"
            )

        overview_text = (
            f"全库({len(entities)}实体, {len(groups)}个类型):\n"
            + "\n".join(f"  {line}" for line in summary_lines)
        )
        self._write_file("overview_all.txt", overview_text)

    def _read_file(self, filename: str) -> str:
        path = CACHE_DIR / filename
        if not path.exists():
            return "知识库暂无数据。"
        try:
            return path.read_text(encoding="utf-8")
        except OSError:
            return "知识库暂无数据。"

    def _write_file(self, filename: str, content: str):
        path = CACHE_DIR / filename
        path.write_text(content, encoding="utf-8")
