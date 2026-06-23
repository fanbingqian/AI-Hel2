"""HEIMDALL Summary Tree Engine (P2.1).

Bucket-seal cascade for knowledge activity summarization:
  L0 (daily)  — 7 days  → 1 L1 weekly summary
  L1 (weekly) — 4 weeks → 1 L2 monthly summary
  L2 (monthly)— 12 months→ 1 L3 yearly summary
  L3 (yearly) — stored indefinitely

LLM-generated summaries are capped at 5000 characters. When a bucket fills,
a new higher-level summary is generated that distills the key entities,
relations, and events from the source period into a compact narrative.

Schema:
  summary_tree(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    level INTEGER NOT NULL,        -- 0=daily, 1=weekly, 2=monthly, 3=yearly
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    content TEXT,                   -- LLM-generated summary
    entity_count INTEGER DEFAULT 0,
    relation_count INTEGER DEFAULT 0,
    source_count INTEGER DEFAULT 0, -- how many lower-level items were summarized
    status TEXT DEFAULT 'pending',  -- pending | complete | stale
    namespace TEXT DEFAULT 'general',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  )
"""

from __future__ import annotations

import logging
from datetime import date, datetime, timedelta
from typing import Callable, Optional

logger = logging.getLogger(__name__)

MAX_SUMMARY_CHARS = 5000

# Bucket thresholds
L0_TO_L1_THRESHOLD = 7    # 7 daily → 1 weekly
L1_TO_L2_THRESHOLD = 4    # 4 weekly → 1 monthly
L2_TO_L3_THRESHOLD = 12   # 12 monthly → 1 yearly

# Level config
LEVEL_NAMES = {0: "daily", 1: "weekly", 2: "monthly", 3: "yearly"}
LEVEL_DURATION_DAYS = {0: 1, 1: 7, 2: 30, 3: 365}
LEVEL_THRESHOLDS = {0: L0_TO_L1_THRESHOLD, 1: L1_TO_L2_THRESHOLD, 2: L2_TO_L3_THRESHOLD}

SUMMARY_TREE_SCHEMA = """
CREATE TABLE IF NOT EXISTS summary_tree (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    level INTEGER NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    content TEXT,
    entity_count INTEGER DEFAULT 0,
    relation_count INTEGER DEFAULT 0,
    source_count INTEGER DEFAULT 0,
    status TEXT DEFAULT 'pending',
    namespace TEXT DEFAULT 'general',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_summary_tree_unique
    ON summary_tree(level, start_date, namespace);
CREATE INDEX IF NOT EXISTS idx_summary_tree_level_status
    ON summary_tree(level, status);
CREATE INDEX IF NOT EXISTS idx_summary_tree_date
    ON summary_tree(start_date);
"""


class SummaryTreeEngine:
    """Manages the bucket-seal cascade for knowledge summaries.

    Usage:
        engine = SummaryTreeEngine(store, llm_call)
        engine.bootstrap_daily(today, namespace="general")
        count = engine.cascade(namespace="general")  # check all levels
    """

    def __init__(self, store, llm_call: Optional[Callable] = None):
        self._store = store
        self._llm_call = llm_call

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def initialize_schema(self) -> None:
        """Create the summary_tree table if it doesn't exist."""
        conn = self._store._conn
        if not conn:
            return
        try:
            conn.executescript(SUMMARY_TREE_SCHEMA)
            conn.commit()
            logger.info("Summary tree schema initialized")
        except Exception as e:
            logger.warning("Summary tree schema init skipped: %s", e)

    def bootstrap_daily(self, target_date: Optional[date] = None,
                        namespace: str = "general") -> Optional[int]:
        """Create a pending L0 (daily) summary entry for the given date.

        Reads the event log for that day to populate entity/relation counts.
        Returns the new summary id, or None if one already exists.
        """
        if target_date is None:
            target_date = date.today()

        conn = self._store._conn
        if not conn:
            return None

        date_str = target_date.isoformat()

        # Skip if already exists
        existing = conn.execute(
            "SELECT id FROM summary_tree WHERE level = 0 AND start_date = ? AND namespace = ?",
            (date_str, namespace),
        ).fetchone()
        if existing:
            return None

        # Count today's event log activity
        entity_count = 0
        relation_count = 0
        try:
            ec = conn.execute(
                "SELECT COUNT(*) FROM kr_event_log WHERE date = ?",
                (date_str,),
            ).fetchone()
            if ec:
                entity_count = ec[0]
        except Exception:
            pass

        try:
            rc = conn.execute(
                "SELECT COUNT(*) FROM kr_relations WHERE date(created_at) = ?",
                (date_str,),
            ).fetchone()
            if rc:
                relation_count = rc[0]
        except Exception:
            pass

        conn.execute(
            "INSERT INTO summary_tree (level, start_date, end_date, entity_count, "
            "relation_count, status, namespace) VALUES (0, ?, ?, ?, ?, 'pending', ?)",
            (date_str, date_str, entity_count, relation_count, namespace),
        )
        conn.commit()

        new_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
        logger.info("Bootstrapped daily summary %d for %s", new_id, date_str)
        return new_id

    def cascade(self, namespace: str = "general") -> dict:
        """Check all levels for cascade opportunities and generate summaries.

        Returns {level: count_of_new_summaries_generated}.
        """
        results: dict[int, int] = {}
        for level in [0, 1, 2]:
            count = self._try_cascade_level(level, namespace)
            if count > 0:
                results[level + 1] = count
        return results

    def generate_summary(self, summary_id: int, system_prompt: str = "",
                         user_prompt: str = "") -> bool:
        """Generate LLM summary for a pending summary_tree entry.

        Returns True if the summary was generated and saved.
        """
        if not self._llm_call:
            logger.debug("No LLM callable configured, skipping summary %d", summary_id)
            return False

        conn = self._store._conn
        if not conn:
            return False

        row = conn.execute(
            "SELECT * FROM summary_tree WHERE id = ? AND status = 'pending'",
            (summary_id,),
        ).fetchone()
        if not row:
            return False

        # Gather source content
        level = row["level"]
        start = row["start_date"]
        end = row["end_date"]

        if level == 0:
            source_text = self._gather_event_log_text(start, end, row["namespace"])
        else:
            source_text = self._gather_child_summaries(level - 1, start, end, row["namespace"])

        if not source_text.strip():
            logger.debug("No source content for summary %d", summary_id)
            return False

        # Build prompts
        sp = system_prompt or (
            "You are a knowledge summarizer. Given a list of knowledge graph events, "
            "produce a concise summary (≤5000 characters) that captures the key entities, "
            "relationships, and themes. Focus on what was learned, created, or changed. "
            "Use bullet points for key items. Write in Chinese."
        )
        up = user_prompt or (
            f"Summarize the following {LEVEL_NAMES.get(level, '')} knowledge activity "
            f"({start} to {end}):\n\n{source_text[:8000]}"
        )

        try:
            content = self._llm_call(sp, up)
            if not content:
                return False

            # Truncate to max chars
            content = content[:MAX_SUMMARY_CHARS]

            conn.execute(
                "UPDATE summary_tree SET content = ?, status = 'complete' WHERE id = ?",
                (content, summary_id),
            )
            conn.commit()
            logger.info("Generated summary for %d (level %d, %s→%s)", summary_id, level, start, end)
            return True
        except Exception as e:
            logger.warning("LLM summary failed for %d: %s", summary_id, e)
            return False

    def get_summaries(self, level: Optional[int] = None,
                      namespace: str = "general",
                      limit: int = 50) -> list[dict]:
        """Fetch summary entries, optionally filtered by level."""
        conn = self._store._conn
        if not conn:
            return []

        if level is not None:
            rows = conn.execute(
                "SELECT * FROM summary_tree WHERE level = ? AND namespace = ? "
                "ORDER BY start_date DESC LIMIT ?",
                (level, namespace, limit),
            ).fetchall()
        else:
            rows = conn.execute(
                "SELECT * FROM summary_tree WHERE namespace = ? "
                "ORDER BY level, start_date DESC LIMIT ?",
                (namespace, limit),
            ).fetchall()
        return [dict(r) for r in rows]

    def get_latest_summary(self, level: int = 1,
                           namespace: str = "general") -> Optional[dict]:
        """Get the most recent completed summary at the given level."""
        conn = self._store._conn
        if not conn:
            return None
        row = conn.execute(
            "SELECT * FROM summary_tree WHERE level = ? AND namespace = ? "
            "AND status = 'complete' ORDER BY end_date DESC LIMIT 1",
            (level, namespace),
        ).fetchone()
        return dict(row) if row else None

    # ------------------------------------------------------------------
    # Internal cascade logic
    # ------------------------------------------------------------------

    def _try_cascade_level(self, level: int, namespace: str) -> int:
        """Check if enough completed entries at `level` exist to create a
        higher-level summary. Returns count of new summaries created."""
        conn = self._store._conn
        if not conn:
            return 0

        threshold = LEVEL_THRESHOLDS.get(level, 0)
        if threshold <= 0:
            return 0

        next_level = level + 1
        level_days = LEVEL_DURATION_DAYS[next_level]

        # Get completed entries at this level that haven't been summarized yet
        rows = conn.execute(
            "SELECT * FROM summary_tree WHERE level = ? AND namespace = ? "
            "AND status = 'complete' "
            "AND id NOT IN ("
            "  SELECT DISTINCT id FROM summary_tree WHERE level = ? AND namespace = ?"
            ") "
            "ORDER BY start_date ASC",
            (level, namespace, next_level, namespace),
        ).fetchall()

        if len(rows) < threshold:
            return 0

        created = 0
        batch: list[dict] = []
        for row in rows:
            batch.append(dict(row))
            if len(batch) >= threshold:
                if self._create_higher_summary(batch, next_level, namespace):
                    created += 1
                batch = []

        # Remaining partial batch — try to summarize if enough
        if len(batch) >= threshold:
            if self._create_higher_summary(batch, next_level, namespace):
                created += 1

        return created

    def _create_higher_summary(self, sources: list[dict], next_level: int,
                               namespace: str) -> bool:
        """Create a pending higher-level summary entry from source summaries."""
        conn = self._store._conn
        if not conn:
            return False

        start_date = min(s["start_date"] for s in sources)
        end_date = max(s["end_date"] for s in sources)

        # Check for existing entry
        existing = conn.execute(
            "SELECT id FROM summary_tree WHERE level = ? AND start_date = ? AND namespace = ?",
            (next_level, start_date, namespace),
        ).fetchone()
        if existing:
            return False

        total_entities = sum(s.get("entity_count", 0) or 0 for s in sources)
        total_relations = sum(s.get("relation_count", 0) or 0 for s in sources)

        conn.execute(
            "INSERT INTO summary_tree (level, start_date, end_date, entity_count, "
            "relation_count, source_count, status, namespace) "
            "VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)",
            (next_level, start_date, end_date, total_entities, total_relations,
             len(sources), namespace),
        )
        conn.commit()

        new_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
        logger.info("Created pending L%d summary %d (%s→%s, %d sources)",
                    next_level, new_id, start_date, end_date, len(sources))

        # Try auto-generate if LLM is available
        if self._llm_call:
            self.generate_summary(new_id)

        return True

    def _gather_event_log_text(self, start: str, end: str, namespace: str) -> str:
        """Extract event descriptions for a date range."""
        conn = self._store._conn
        if not conn:
            return ""

        try:
            rows = conn.execute(
                "SELECT description, event_type, entity_id FROM kr_event_log "
                "WHERE date BETWEEN ? AND ? ORDER BY date, timestamp LIMIT 200",
                (start, end),
            ).fetchall()
            if not rows:
                return ""

            lines: list[str] = []
            for r in rows:
                desc = r["description"] or ""
                if desc:
                    lines.append(f"- [{r['event_type']}] {desc}")
            return "\n".join(lines)
        except Exception:
            return ""

    def _gather_child_summaries(self, child_level: int, start: str, end: str,
                                namespace: str) -> str:
        """Gather content from child-level summaries in the date range."""
        conn = self._store._conn
        if not conn:
            return ""

        try:
            rows = conn.execute(
                "SELECT content, start_date, end_date, entity_count, relation_count "
                "FROM summary_tree WHERE level = ? AND namespace = ? "
                "AND start_date >= ? AND end_date <= ? AND status = 'complete' "
                "ORDER BY start_date",
                (child_level, namespace, start, end),
            ).fetchall()

            parts: list[str] = []
            for r in rows:
                content = r["content"] or ""
                if content:
                    header = f"## {r['start_date']} ~ {r['end_date']} ({r['entity_count']} entities, {r['relation_count']} relations)"
                    parts.append(f"{header}\n\n{content}")
            return "\n\n".join(parts)
        except Exception:
            return ""
