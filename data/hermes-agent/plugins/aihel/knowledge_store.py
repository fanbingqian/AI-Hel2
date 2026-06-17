"""Read-only accessor for AI-Hel's knowledge_cache.db.

Uses thread-local SQLite connections with WAL + query_only mode
to avoid conflicting with AI-Hel's Rust-side writes."""

from __future__ import annotations

import sqlite3
import threading
from pathlib import Path
from typing import Dict, List, Optional, Tuple


class KnowledgeStore:
    """Read-only accessor for AI-Hel's knowledge_cache.db."""

    def __init__(self, db_path: Path):
        self._db_path = db_path
        self._local = threading.local()

    def _get_conn(self) -> sqlite3.Connection:
        """Get or create a thread-local read-only connection."""
        if not hasattr(self._local, "conn") or self._local.conn is None:
            conn = sqlite3.connect(str(self._db_path))
            conn.row_factory = sqlite3.Row
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA query_only=ON")
            self._local.conn = conn
        return self._local.conn

    # ── Entity queries ──────────────────────────────────────────

    def search_entities(
        self, query: str, entity_type: Optional[str], limit: int
    ) -> List[Dict]:
        """FTS5 full-text search → LIKE fallback.

        Returns [{id, name, entity_type, description, confidence, aliases, source_file}, ...]
        """
        conn = self._get_conn()
        results: List[Dict] = []

        # 1) Try FTS5 prefix search
        try:
            fts_query = " ".join(f'"{t}"*' for t in query.split() if t)
            if fts_query:
                sql = (
                    "SELECT e.id, e.name, e.entity_type, e.description, "
                    "  e.confidence, e.aliases, e.source_file "
                    "FROM cache_entities_fts f "
                    "JOIN cache_entities e ON e.rowid = f.rowid "
                    "WHERE cache_entities_fts MATCH ? "
                    "ORDER BY rank LIMIT ?"
                )
                rows = conn.execute(sql, (fts_query, limit)).fetchall()
                results = [dict(r) for r in rows]
        except Exception:
            pass

        # 2) LIKE fallback
        if not results:
            like = f"%{query}%"
            sql = (
                "SELECT id, name, entity_type, description, confidence, "
                "  aliases, source_file "
                "FROM cache_entities "
                "WHERE hidden = 0 AND (name LIKE ? OR description LIKE ? OR aliases LIKE ?) "
                "ORDER BY confidence DESC LIMIT ?"
            )
            rows = conn.execute(sql, (like, like, like, limit)).fetchall()
            results = [dict(r) for r in rows]

        # 3) Type filter
        if entity_type and results:
            results = [r for r in results if r["entity_type"] == entity_type]

        return results

    def get_entity(self, entity_id: str) -> Optional[Dict]:
        """Fetch a single entity by ID."""
        conn = self._get_conn()
        row = conn.execute(
            "SELECT * FROM cache_entities WHERE id = ? AND hidden = 0",
            (entity_id,),
        ).fetchone()
        return dict(row) if row else None

    def get_entity_relations(self, entity_id: str) -> Tuple[List[Dict], List[Dict]]:
        """Return (inbound_relations, outbound_relations) with entity names joined."""
        conn = self._get_conn()

        inbound = conn.execute(
            "SELECT r.*, e.name AS from_name, e.entity_type AS from_type "
            "FROM cache_relations r "
            "JOIN cache_entities e ON e.id = r.from_id "
            "WHERE r.to_id = ?",
            (entity_id,),
        ).fetchall()

        outbound = conn.execute(
            "SELECT r.*, e.name AS to_name, e.entity_type AS to_type "
            "FROM cache_relations r "
            "JOIN cache_entities e ON e.id = r.to_id "
            "WHERE r.from_id = ?",
            (entity_id,),
        ).fetchall()

        return [dict(r) for r in inbound], [dict(r) for r in outbound]

    def get_entity_count(self) -> int:
        """Count non-hidden entities."""
        conn = self._get_conn()
        row = conn.execute(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0"
        ).fetchone()
        return row[0] if row else 0

    # ── Recent changes ──────────────────────────────────────────

    def get_recent_changes(self, since_iso: str, limit: int = 10) -> List[Dict]:
        """Entities added or updated since `since_iso`, ordered by recency."""
        conn = self._get_conn()
        rows = conn.execute(
            "SELECT id, name, entity_type, description, confidence, "
            "  created_at, updated_at "
            "FROM cache_entities "
            "WHERE hidden = 0 AND updated_at > ? "
            "ORDER BY updated_at DESC LIMIT ?",
            (since_iso, limit),
        ).fetchall()
        return [dict(r) for r in rows]

    # ── Knowledge snapshot ───────────────────────────────────────

    def get_knowledge_snapshot(self) -> str:
        """Read KNOWLEDGE.md or assemble a summary from the database.

        Kept under 800 characters to preserve prompt cache space."""
        md_path = self._db_path.parent / "KNOWLEDGE.md"
        if md_path.exists():
            try:
                content = md_path.read_text(encoding="utf-8")
                if len(content) > 800:
                    content = content[:800].rsplit("\n", 1)[0] + "\n..."
                return content
            except Exception:
                pass
        return self._assemble_snapshot_from_db()

    def _assemble_snapshot_from_db(self) -> str:
        """Sample top-3 entities per type by confidence."""
        conn = self._get_conn()
        types = conn.execute(
            "SELECT DISTINCT entity_type FROM cache_entities WHERE hidden = 0"
        ).fetchall()
        lines: List[str] = []
        for (etype,) in types:
            rows = conn.execute(
                "SELECT name, description FROM cache_entities "
                "WHERE entity_type = ? AND hidden = 0 "
                "ORDER BY confidence DESC LIMIT 3",
                (etype,),
            ).fetchall()
            for r in rows:
                desc = (r["description"] or "")[:60]
                lines.append(f"- {r['name']} ({etype}): {desc}")
        return "\n".join(lines) if lines else "(知识库为空)"

    # ── Stats ────────────────────────────────────────────────────

    def get_stats(self) -> Dict:
        """Return entity count, relation count, type breakdown."""
        conn = self._get_conn()
        total_e = conn.execute(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0"
        ).fetchone()[0]
        total_r = conn.execute(
            "SELECT COUNT(*) FROM cache_relations"
        ).fetchone()[0]
        type_breakdown = [
            dict(r)
            for r in conn.execute(
                "SELECT entity_type, COUNT(*) AS count "
                "FROM cache_entities WHERE hidden = 0 "
                "GROUP BY entity_type"
            ).fetchall()
        ]
        return {
            "total_entities": total_e,
            "total_relations": total_r,
            "type_breakdown": type_breakdown,
        }
