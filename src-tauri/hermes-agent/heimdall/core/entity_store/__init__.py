"""Knowledge Ring Entity Store — unified entry point (V2.2).

Backward-compatible with the original EntityStore interface.
Internally delegates to submodules: schema, ops, archive, crypto, migration.
"""

import json
import logging
import sqlite3
import threading
from pathlib import Path
from typing import Any, Optional

from .archive import ArchiveManager
from .migration import MigrationRunner
from .ops import EntityStoreOps
from .schema import (
    ENTITY_TYPE_COLORS,
    ENTITY_TYPE_MIGRATION_MAP,
    EVOLUTION_ALTER_SQL,
    EVOLUTION_TABLES_SQL,
    KNOWLEDGE_RING_FTS_SQL,
    KNOWLEDGE_RING_SCHEMA,
    LEGACY_FTS_SQL,
    LEGACY_HEIMDALL_ENTITY_SCHEMA,
    MEMORY_EMBEDDING_BYTES,
    MEMORY_EMBEDDING_SPEC,
    VALID_ENTITY_TYPES,
    VALID_RELATION_TYPES,
    migrate_importance,
    migrate_namespace,
)

logger = logging.getLogger(__name__)

from .schema import (
    VALID_EVENT_LOG_TYPES,
    VALID_PROFILE_TYPES,
    VALID_RELATION_DIRECTIONS,
    VALID_TYPE_DETAILS,
)

# Re-export for backward compatibility
VALID_ENTITY_TYPES_V2 = VALID_ENTITY_TYPES
VALID_RELATION_TYPES_V2 = VALID_RELATION_TYPES

# Legacy re-exports from old entity_store.py
HEIMDALL_ENTITY_SCHEMA = LEGACY_HEIMDALL_ENTITY_SCHEMA
HEIMDALL_FTS_SQL = LEGACY_FTS_SQL

# For backward compat with extraction.py
VALID_ENTITY_TYPES_OLD = frozenset({
    "person", "organization", "project", "tool",
    "concept", "skill", "event", "location", "media",
})
VALID_STATUSES = frozenset({"active", "dormant"})
VALID_SOURCE_TRACKS = frozenset({"memory", "knowledge", "both"})


class EntityStore:
    """Backward-compatible EntityStore (V2.2 delegates to submodules).

    Usage:
        store = EntityStore(db_path, salt="abc123...")
        store.initialize()
        entity_id = store.upsert_entity(display_name="Alice", entity_type="person")
        results = store.search_entities("Alice")
    """

    def __init__(self, db_path: Path, salt: str = ""):
        self.db_path = db_path
        self.salt = salt
        self._lock = threading.Lock()
        self._conn: Optional[sqlite3.Connection] = None

        # Sub-modules (initialized in initialize())
        self.ops: Optional[EntityStoreOps] = None
        self.archive: Optional[ArchiveManager] = None
        self.migration: Optional[MigrationRunner] = None

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def _ensure_v2_columns(self) -> None:
        """Add V2.2 columns that may be missing from older-schema databases.

        SQLite CREATE TABLE IF NOT EXISTS won't add columns to existing tables,
        so we must ALTER TABLE for each missing column before running schema DDL
        that references them via indexes.
        """
        if not self._conn:
            return

        migrations = {
            "kr_entities": [
                ("types", "TEXT DEFAULT '[\"concept\"]'"),
                ("memory_embedding", "BLOB"),
            ],
            "kr_aliases": [
                ("confidence", "REAL DEFAULT 0.5"),
                ("last_seen", "TIMESTAMP DEFAULT CURRENT_TIMESTAMP"),
            ],
            "kr_originals": [
                ("compressed_content", "BLOB"),
                ("local_ttl", "INTEGER DEFAULT 7"),
                ("archive_status", "TEXT DEFAULT 'local'"),
                ("archived_at", "TIMESTAMP"),
            ],
            "kr_profiles": [
                ("content_encrypted", "BLOB"),
                ("ttl_days", "INTEGER DEFAULT NULL"),
                ("expires_at", "TIMESTAMP"),
            ],
        }

        for table, cols in migrations.items():
            existing = {
                c["name"] for c in
                self._conn.execute(f"PRAGMA table_info({table})").fetchall()
            }
            if not existing:
                continue  # table doesn't exist yet, KNOWLEDGE_RING_SCHEMA will create it
            for col_name, col_def in cols:
                if col_name not in existing:
                    self._conn.execute(
                        f"ALTER TABLE {table} ADD COLUMN {col_name} {col_def}"
                    )
                    logger.info("Added column %s.%s", table, col_name)

        # Migrate data: 'type' → 'types'
        kr_cols = {
            c["name"] for c in
            self._conn.execute("PRAGMA table_info(kr_entities)").fetchall()
        }
        if "type" in kr_cols and "types" in kr_cols:
            self._conn.execute(
                "UPDATE kr_entities SET types = json_array(type) "
                "WHERE (types IS NULL OR types = '' OR types = '[]' OR types = '[\"concept\"]')"
            )

        # Encrypt existing plaintext profiles
        prof_cols = {
            c["name"] for c in
            self._conn.execute("PRAGMA table_info(kr_profiles)").fetchall()
        }
        if "content" in prof_cols and "content_encrypted" in prof_cols:
            from .crypto import device_encrypt
            rows = self._conn.execute(
                "SELECT profile_id, content FROM kr_profiles "
                "WHERE content_encrypted IS NULL AND content IS NOT NULL AND content != ''"
            ).fetchall()
            for row in rows:
                encrypted = device_encrypt(row["content"])
                self._conn.execute(
                    "UPDATE kr_profiles SET content_encrypted = ? WHERE profile_id = ?",
                    (encrypted, row["profile_id"]),
                )
            if rows:
                logger.info("Encrypted %d plaintext profiles", len(rows))

    def _ensure_v2_3_columns(self) -> None:
        """Add V2.3 namespace columns and run namespace migration if needed."""
        if not self._conn:
            return

        # Check migration status
        status_row = self._conn.execute(
            "SELECT value FROM schema_meta WHERE key = 'migration_status'"
        ).fetchone()
        status = status_row["value"] if status_row else None

        if status in ("in_progress", "failed"):
            failed_table = self._conn.execute(
                "SELECT value FROM schema_meta WHERE key = 'failed_at_table'"
            ).fetchone()
            detail = f" at table {failed_table['value']}" if failed_table else ""
            logger.error(
                "Namespace migration status is '%s'%s. Skipping business logic.",
                status, detail,
            )
            return

        if status == "completed":
            return

        # Run V2.3 namespace migration
        try:
            migrate_namespace(self._conn, str(self.db_path))
            logger.info("V2.3 namespace migration completed successfully")
        except RuntimeError as e:
            logger.error("V2.3 namespace migration failed: %s", e)

    def _ensure_v2_4_columns(self) -> None:
        """Add V2.4 importance columns and run importance migration (P2.3)."""
        if not self._conn:
            return

        status_row = self._conn.execute(
            "SELECT value FROM schema_meta WHERE key = 'importance_migration'"
        ).fetchone()
        status = status_row["value"] if status_row else None

        if status == "completed":
            return

        if status == "failed":
            err_row = self._conn.execute(
                "SELECT value FROM schema_meta WHERE key = 'importance_migration_error'"
            ).fetchone()
            detail = f": {err_row['value']}" if err_row else ""
            logger.error("Importance migration previously failed%s. Skipping.", detail)
            return

        try:
            migrate_importance(self._conn, str(self.db_path))
            logger.info("V2.4 importance migration completed successfully")
        except RuntimeError as e:
            logger.error("V2.4 importance migration failed: %s", e)

    def _ensure_v3_0_columns(self) -> None:
        """Create Phase 1-5 evolution tables and run ALTER extensions."""
        if not self._conn:
            return

        status_row = self._conn.execute(
            "SELECT value FROM schema_meta WHERE key = 'evolution_tables_v3'"
        ).fetchone()
        if status_row and status_row["value"] == "completed":
            return

        # Create new tables
        try:
            self._conn.executescript(EVOLUTION_TABLES_SQL)
        except Exception as e:
            logger.warning("Evolution tables creation: %s", e)

        # Run ALTER statements (ignore "duplicate column" errors)
        for alter_sql in EVOLUTION_ALTER_SQL:
            try:
                self._conn.execute(alter_sql)
            except Exception:
                pass  # column already exists

        self._conn.execute(
            "INSERT OR REPLACE INTO schema_meta VALUES ('evolution_tables_v3', 'completed')"
        )
        logger.info("V3.0 evolution tables created")

    def initialize(self) -> None:
        """Open database and create tables if needed."""
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(
            str(self.db_path),
            check_same_thread=False,
            timeout=1.0,
            isolation_level=None,
        )
        self._conn.row_factory = sqlite3.Row
        self._conn.execute("PRAGMA journal_mode=WAL")
        self._conn.execute("PRAGMA foreign_keys=ON")

        # Create legacy tables
        self._conn.executescript(LEGACY_HEIMDALL_ENTITY_SCHEMA)
        self._conn.executescript(LEGACY_FTS_SQL)

        # Ensure V2.2 columns exist before running schema (handles old-schema DBs)
        self._ensure_v2_columns()

        # Create Knowledge Ring V2.2 tables (must run BEFORE V2.3 migration
        # so that kr_* tables exist for namespace column migration)
        self._conn.executescript(KNOWLEDGE_RING_SCHEMA)
        self._conn.executescript(KNOWLEDGE_RING_FTS_SQL)

        # Ensure schema_meta table exists for migration tracking
        self._conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT)"
        )

        # V2.3 namespace migration (adds namespace columns to old-schema DBs)
        self._ensure_v2_3_columns()

        # V2.4 importance columns migration (P2.3)
        self._ensure_v2_4_columns()

        # V3.0 evolution tables (Phase 1-5 heimdall-implementation-plan)
        self._ensure_v3_0_columns()

        # Initialize sub-modules
        self.ops = EntityStoreOps(self._conn, self._lock)
        self.archive = ArchiveManager(self._conn)
        self.migration = MigrationRunner(self._conn, self._lock)

        # Run migrations
        self.migration.migrate_all()

    def close(self) -> None:
        if self._conn:
            self._conn.close()
            self._conn = None
            self.ops = None
            self.archive = None
            self.migration = None

    # ------------------------------------------------------------------
    # Legacy Entity CRUD (delegate to old table methods)
    # ------------------------------------------------------------------

    def upsert_entity(
        self,
        display_name: str,
        entity_type: str,
        source_session_id: str = "",
        source_track: str = "memory",
        confidence: float = 0.5,
        attributes: Optional[dict] = None,
    ) -> str:
        import time as _time
        now = _time.time()

        def _do(conn):
            row = conn.execute(
                "SELECT entity_id, occurrence_count FROM heimdall_entities "
                "WHERE display_name = ? AND entity_type = ? AND status = 'active'",
                (display_name, entity_type),
            ).fetchone()

            if row:
                eid = row["entity_id"]
                conn.execute(
                    "UPDATE heimdall_entities SET "
                    "last_seen_at = ?, occurrence_count = occurrence_count + 1, "
                    "confidence = MAX(confidence, ?), attributes_json = ? "
                    "WHERE entity_id = ?",
                    (now, confidence,
                     json.dumps(attributes or {}, ensure_ascii=False), eid),
                )
                return eid

            import uuid as _uuid
            from heimdall.core.privacy import hash_name, is_third_party

            eid = _uuid.uuid4().hex
            salted = hash_name(display_name, self.salt) if is_third_party(entity_type) else None
            conn.execute(
                "INSERT INTO heimdall_entities "
                "(entity_id, entity_type, display_name, salted_hash, attributes_json, "
                "first_seen_at, last_seen_at, confidence, source_session_id, source_track) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (eid, entity_type, display_name, salted,
                 json.dumps(attributes or {}, ensure_ascii=False),
                 now, now, confidence, source_session_id, source_track),
            )
            return eid

        return self._execute_write(_do)

    def get_entity(self, entity_id: str) -> Optional[dict]:
        if not self._conn:
            return None
        row = self._conn.execute(
            "SELECT * FROM heimdall_entities WHERE entity_id = ?", (entity_id,)
        ).fetchone()
        return dict(row) if row else None

    def get_entity_by_name(self, display_name: str, entity_type: Optional[str] = None) -> Optional[dict]:
        if not self._conn:
            return None
        if entity_type:
            row = self._conn.execute(
                "SELECT * FROM heimdall_entities "
                "WHERE display_name = ? AND entity_type = ? AND status = 'active'",
                (display_name, entity_type),
            ).fetchone()
        else:
            row = self._conn.execute(
                "SELECT * FROM heimdall_entities "
                "WHERE display_name = ? AND status = 'active'",
                (display_name,),
            ).fetchone()
        return dict(row) if row else None

    def search_entities(self, query: str, limit: int = 10) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        from .ops import _sanitize_fts_query
        sanitized = _sanitize_fts_query(query)
        try:
            rows = self._conn.execute(
                "SELECT e.* FROM heimdall_entities e "
                "JOIN heimdall_entities_fts fts ON e.rowid = fts.rowid "
                "WHERE heimdall_entities_fts MATCH ? AND e.status = 'active' "
                "ORDER BY rank LIMIT ?",
                (sanitized, limit),
            ).fetchall()
        except sqlite3.OperationalError:
            return []
        return [dict(r) for r in rows]

    def list_entities(
        self, entity_type: Optional[str] = None, source_track: Optional[str] = None,
        limit: int = 50, offset: int = 0,
    ) -> list[dict]:
        if not self._conn:
            return []
        clauses = ["status = 'active'"]
        params: list[Any] = []
        if entity_type:
            clauses.append("entity_type = ?")
            params.append(entity_type)
        if source_track:
            clauses.append("source_track = ?")
            params.append(source_track)
        where = " AND ".join(clauses)
        rows = self._conn.execute(
            f"SELECT * FROM heimdall_entities WHERE {where} "
            "ORDER BY last_seen_at DESC LIMIT ? OFFSET ?",
            (*params, limit, offset),
        ).fetchall()
        return [dict(r) for r in rows]

    def get_entity_count(self) -> int:
        if not self._conn:
            return 0
        row = self._conn.execute(
            "SELECT COUNT(*) as cnt FROM heimdall_entities WHERE status = 'active'"
        ).fetchone()
        return row["cnt"] if row else 0

    def set_entity_status(self, entity_id: str, status: str) -> bool:
        if status not in VALID_STATUSES:
            raise ValueError(f"Invalid status: {status}")

        def _do(conn):
            cur = conn.execute(
                "UPDATE heimdall_entities SET status = ? WHERE entity_id = ?",
                (status, entity_id),
            )
            return cur.rowcount > 0

        return self._execute_write(_do)

    # ------------------------------------------------------------------
    # Legacy Memory Edges
    # ------------------------------------------------------------------

    def add_memory_edge(
        self, entity_id: str, role: str, memory_id: str = "",
        emotion: Optional[float] = None, session_id: str = "",
        ner_confidence: float = 1.0,
    ) -> str:
        import uuid as _uuid
        import time as _time
        edge_id = _uuid.uuid4().hex
        now = _time.time()

        def _do(conn):
            conn.execute(
                "INSERT INTO heimdall_memory_edges "
                "(id, memory_id, entity_id, role, emotion, timestamp, session_id, ner_confidence) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (edge_id, memory_id, entity_id, role, emotion, now, session_id, ner_confidence),
            )
            return edge_id

        return self._execute_write(_do)

    # ------------------------------------------------------------------
    # Legacy Social Graph
    # ------------------------------------------------------------------

    def upsert_social_edge(
        self, source_entity_id: str, target_entity_id: str,
        relationship_type: str = "", emotion: float = 0.0,
    ) -> None:
        import time as _time
        now = _time.time()

        def _do(conn):
            row = conn.execute(
                "SELECT id, intensity, valence, volatility, evidence_count "
                "FROM heimdall_social_graph "
                "WHERE source_entity_id = ? AND target_entity_id = ? AND relationship_type = ?",
                (source_entity_id, target_entity_id, relationship_type),
            ).fetchone()

            if row:
                n = row["evidence_count"] + 1
                new_valence = (row["valence"] * row["evidence_count"] + emotion) / n
                old_sq = (row["valence"] ** 2 + row["volatility"] ** 2) * row["evidence_count"]
                new_volatility = ((old_sq + emotion ** 2) / n - new_valence ** 2) ** 0.5
                new_intensity = 1.0 / (1.0 + 2.71828 ** (-0.1 * n))
                health = (0.5 + 0.5 * new_valence) * (1.0 - min(new_volatility, 1.0))
                conn.execute(
                    "UPDATE heimdall_social_graph SET "
                    "intensity = ?, valence = ?, volatility = ?, health_score = ?, "
                    "last_seen = ?, evidence_count = ? WHERE id = ?",
                    (new_intensity, new_valence, new_volatility, health, now, n, row["id"]),
                )
            else:
                conn.execute(
                    "INSERT INTO heimdall_social_graph "
                    "(source_entity_id, target_entity_id, relationship_type, "
                    "intensity, valence, volatility, health_score, first_seen, last_seen, evidence_count) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
                    (source_entity_id, target_entity_id, relationship_type,
                     0.1, emotion, 0.0, 0.5, now, now),
                )

        self._execute_write(_do)

    def get_social_edges(self, entity_id: str) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM heimdall_social_graph "
            "WHERE source_entity_id = ? OR target_entity_id = ? "
            "ORDER BY intensity DESC",
            (entity_id, entity_id),
        ).fetchall()
        return [dict(r) for r in rows]

    def get_reconnect_suggestions(self, inactivity_seconds: int = 7776000) -> list[dict]:
        if not self._conn:
            return []
        import time as _time
        cutoff = _time.time() - inactivity_seconds
        rows = self._conn.execute(
            "SELECT s.*, e.display_name as target_name, e.entity_type as target_type "
            "FROM heimdall_social_graph s "
            "JOIN heimdall_entities e ON s.target_entity_id = e.entity_id "
            "WHERE s.last_seen < ? AND s.evidence_count >= 10 "
            "ORDER BY s.intensity DESC LIMIT 5",
            (cutoff,),
        ).fetchall()
        return [dict(r) for r in rows]

    # ------------------------------------------------------------------
    # Legacy Knowledge entries
    # ------------------------------------------------------------------

    def search_knowledge(self, query: str, domain: Optional[str] = None, limit: int = 10) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        from .ops import _sanitize_fts_query
        sanitized = _sanitize_fts_query(query)
        try:
            if domain:
                rows = self._conn.execute(
                    "SELECT k.* FROM heimdall_knowledge_entries k "
                    "JOIN heimdall_knowledge_fts fts ON k.rowid = fts.rowid "
                    "WHERE heimdall_knowledge_fts MATCH ? AND k.domain = ? "
                    "ORDER BY rank LIMIT ?",
                    (sanitized, domain, limit),
                ).fetchall()
            else:
                rows = self._conn.execute(
                    "SELECT k.* FROM heimdall_knowledge_entries k "
                    "JOIN heimdall_knowledge_fts fts ON k.rowid = fts.rowid "
                    "WHERE heimdall_knowledge_fts MATCH ? "
                    "ORDER BY rank LIMIT ?",
                    (sanitized, limit),
                ).fetchall()
        except sqlite3.OperationalError:
            return []
        return [dict(r) for r in rows]

    def upsert_knowledge(
        self,
        domain: str = "general",
        title: str = "",
        content: str = "",
        mastery_level: str = "了解",
        confidence: float = 0.5,
        source_session_id: str = "",
    ) -> Optional[str]:
        """Insert or update a knowledge entry. Returns entry_id."""
        if not self._conn:
            return None
        import uuid
        import time
        # Try to find existing entry by domain + title
        row = self._conn.execute(
            "SELECT entry_id FROM heimdall_knowledge_entries WHERE domain = ? AND title = ?",
            (domain, title),
        ).fetchone()
        if row:
            entry_id = row["entry_id"]
            self._conn.execute(
                "UPDATE heimdall_knowledge_entries SET content=?, mastery_level=?, "
                "confidence=?, updated_at=CURRENT_TIMESTAMP WHERE entry_id=?",
                (content, mastery_level, confidence, entry_id),
            )
        else:
            entry_id = f"k_{uuid.uuid4().hex[:12]}"
            self._conn.execute(
                "INSERT INTO heimdall_knowledge_entries "
                "(entry_id, domain, title, content, mastery_level, confidence, source_session_id) "
                "VALUES (?, ?, ?, ?, ?, ?, ?)",
                (entry_id, domain, title, content, mastery_level, confidence, source_session_id),
            )
        return entry_id

    def touch_knowledge(self, entry_id: str) -> None:
        """Update last_accessed_at for a knowledge entry."""
        if not self._conn:
            return
        try:
            self._conn.execute(
                "UPDATE heimdall_knowledge_entries SET last_accessed_at=CURRENT_TIMESTAMP "
                "WHERE entry_id=?",
                (entry_id,),
            )
        except Exception:
            pass

    def get_stale_knowledge(self, days: int = 30) -> list[dict]:
        """Get knowledge entries not accessed in N days."""
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM heimdall_knowledge_entries "
            "WHERE last_accessed_at < datetime('now', ?) "
            "ORDER BY last_accessed_at ASC",
            (f"-{days} days",),
        ).fetchall()
        return [dict(r) for r in rows]

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — entity methods (delegate to ops)
    # ------------------------------------------------------------------

    def upsert_entity_v2(self, name: str, entity_type: str = "concept",
                         type_detail: str = "", domains: Optional[list] = None,
                         properties: Optional[dict] = None,
                         tags: Optional[list] = None,
                         confidence: float = 0.5, source_ref: Optional[str] = None,
                         session_id: str = "", namespace: str = "general",
                         **kwargs) -> str:
        """Insert/update entity in kr_entities (V2.3: + namespace)."""
        types = kwargs.pop("types", [entity_type] if entity_type else ["concept"])
        if isinstance(types, str):
            try:
                types = json.loads(types)
            except (json.JSONDecodeError, TypeError):
                types = [entity_type] if entity_type else ["concept"]
        memory_embedding = kwargs.pop("memory_embedding", None)
        hrr_vector = kwargs.pop("hrr_vector", None)
        return self.ops.upsert_entity(
            name=name, types=types, type_detail=type_detail,
            domains=domains, properties=properties, tags=tags,
            confidence=confidence, hrr_vector=hrr_vector,
            memory_embedding=memory_embedding, session_id=session_id,
            namespace=namespace,
        )

    def get_entity_v2(self, entity_id: str) -> Optional[dict]:
        return self.ops.get_entity(entity_id)

    def get_entity_v2_by_name(self, name: str, entity_type: Optional[str] = None) -> Optional[dict]:
        entity = self.ops.get_entity_by_name(name)
        if entity and entity_type:
            types_list = entity.get("types", [])
            if entity_type not in types_list:
                return None
        return entity

    def list_entities_v2(self, entity_type: Optional[str] = None,
                         domain: Optional[str] = None,
                         limit: int = 50, offset: int = 0) -> list[dict]:
        return self.ops.list_entities(entity_type=entity_type, domain=domain,
                                      limit=limit, offset=offset)

    def search_entities_v2(self, query: str, limit: int = 10) -> list[dict]:
        return self.ops.search_entities(query, limit)

    def update_entity_v2(self, entity_id: str, **fields) -> bool:
        return self.ops.update_entity(entity_id, **fields)

    def get_entity_count_v2(self) -> int:
        return self.ops.get_entity_count()

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — aliases (delegate to ops)
    # ------------------------------------------------------------------

    def add_alias(self, entity_id: str, name: str, context: str = "",
                  confidence: float = 0.5, session_id: str = "") -> int:
        return self.ops.add_alias(entity_id, name, context=context,
                                  confidence=confidence, session_id=session_id)

    def get_aliases(self, entity_id: str) -> list[dict]:
        return self.ops.get_aliases(entity_id)

    def search_aliases(self, query: str, limit: int = 10) -> list[dict]:
        return self.ops.search_aliases(query, limit)

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — relations (delegate to ops)
    # ------------------------------------------------------------------

    def add_relation(self, source_id: str, target_id: str, rel_type: str,
                     confidence: float = 0.5, direction: str = "bidirectional",
                     source_text: str = "", session_id: str = "",
                     namespace: str = "general") -> int:
        return self.ops.add_relation(source_id, target_id, rel_type,
                                     confidence=confidence, direction=direction,
                                     source_text=source_text, session_id=session_id,
                                     namespace=namespace)

    def get_relations(self, entity_id: str) -> list[dict]:
        return self.ops.get_relations(entity_id)

    def find_relation_path(self, source_id: str, target_id: str, max_depth: int = 2) -> list[dict]:
        return self.ops.find_relation_path(source_id, target_id, max_depth)

    def search_relations(self, query: str, limit: int = 10) -> list[dict]:
        return self.ops.search_relations(query, limit)

    def get_relation_count(self) -> int:
        return self.ops.get_relation_count()

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — originals (delegate to ops + archive)
    # ------------------------------------------------------------------

    def archive_original(self, source_type: str, content: str,
                         metadata: Optional[dict] = None, session_id: str = "",
                         namespace: str = "general") -> int:
        return self.ops.add_original(content=content, source_type=source_type,
                                     metadata=metadata, session_id=session_id,
                                     namespace=namespace)

    def search_originals(self, query: str, limit: int = 5) -> list[dict]:
        return self.ops.search_originals(query, limit)

    def get_original(self, original_id: int) -> Optional[dict]:
        return self.ops.get_original(original_id)

    def retrieve_original(self, original_id: int) -> Optional[str]:
        return self.archive.retrieve_original(original_id)

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — profiles (delegate to ops, encrypted)
    # ------------------------------------------------------------------

    def upsert_profile(self, profile_type: str, content: str,
                       trigger_context: str = "", confidence: float = 0.5,
                       tag: str = "ai_only", ttl_days: Optional[int] = None,
                       session_id: str = "") -> int:
        return self.ops.add_profile(
            profile_type=profile_type, content=content,
            trigger_context=trigger_context, confidence=confidence,
            tag=tag, ttl_days=ttl_days, session_id=session_id,
        )

    def search_profiles(self, query: str, profile_type: Optional[str] = None,
                        limit: int = 10) -> list[dict]:
        return self.ops.search_profiles(query, profile_type, limit)

    def get_profiles_by_tag(self, tag: str = "ai_only") -> list[dict]:
        return self.ops.get_profiles_by_tag(tag)

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — event log
    # ------------------------------------------------------------------

    def get_daily_events(self, target_date: Optional[str] = None) -> list[dict]:
        return self.ops.get_daily_events(target_date)

    def get_event_date_range(self, start_date: str, end_date: str) -> list[dict]:
        return self.ops.get_event_date_range(start_date, end_date)

    def calculate_streak(self) -> int:
        return self.ops.calculate_streak()

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — domains
    # ------------------------------------------------------------------

    def register_domain(self, domain_name: str, first_entity_id: str = "",
                        status: str = "auto_created") -> None:
        self.ops.register_domain(domain_name, first_entity_id, status)

    def get_domains(self) -> list[dict]:
        return self.ops.get_domains()

    def get_domain_stats(self) -> list[dict]:
        return self.ops.get_domain_stats()

    # ------------------------------------------------------------------
    # Knowledge Ring V2.2 — FTS multi-search + ring graph
    # ------------------------------------------------------------------

    def search_fts_multi(self, query: str, limit: int = 10) -> dict:
        return self.ops.search_fts_multi(query, limit)

    def get_ring_graph_data(self) -> dict:
        return self.ops.get_ring_graph_data()

    # ------------------------------------------------------------------
    # Knowledge Ring V2.3 — pending sync queue (unified retry)
    # ------------------------------------------------------------------

    def insert_pending_sync(self, payload_type: str, payload: dict,
                            namespace: str = "general") -> str:
        """Enqueue a failed sync for background retry (unified Provider↔HTTP path)."""
        return self.ops.insert_pending_sync(payload_type, payload, namespace)

    def list_pending_sync(self, namespace: str = None, limit: int = 50) -> list[dict]:
        """List retryable pending sync records (retry_count < 5)."""
        return self.ops.list_pending_sync(namespace, limit)

    def delete_pending_sync(self, pending_id: str) -> None:
        """Manually remove a pending record (user-confirmed discard)."""
        self.ops.delete_pending_sync(pending_id)

    def cleanup_dead_pending_sync(self, retention_days: int = 7) -> int:
        """Purge dead rows (retry_count >= 5) older than retention_days. Returns deleted count."""
        return self.ops.cleanup_dead_pending_sync(retention_days)

    def get_pending_failures(self, namespace: str = None) -> list[dict]:
        """List failed pending records (retry_count >= 5) for UI visibility."""
        return self.ops.get_pending_failures(namespace)

    # ------------------------------------------------------------------
    # V2.4 Importance scoring (P2.3)
    # ------------------------------------------------------------------

    def recalculate_all_importance(self, namespace: Optional[str] = None) -> int:
        """Recalculate importance for all entities. Returns count of updated entities."""
        from .importance import ImportanceEngine
        engine = ImportanceEngine(self)
        return engine.recalc_all(namespace=namespace)

    def get_high_importance_entities(
        self, limit: int = 20, namespace: Optional[str] = None
    ) -> list[dict]:
        """Return top-N entities by importance score."""
        from .importance import ImportanceEngine
        engine = ImportanceEngine(self)
        return engine.get_top_entities(limit=limit, namespace=namespace)

    def get_importance_level_counts(
        self, namespace: Optional[str] = None
    ) -> dict[str, int]:
        """Return entity counts grouped by importance level."""
        from .importance import ImportanceEngine
        engine = ImportanceEngine(self)
        return engine.get_level_counts(namespace=namespace)

    # ------------------------------------------------------------------
    # V2.5 Summary tree (P2.1)
    # ------------------------------------------------------------------

    def summary_tree_init_schema(self) -> None:
        """Ensure the summary_tree table exists."""
        from .summary_tree import SummaryTreeEngine
        engine = SummaryTreeEngine(self)
        engine.initialize_schema()

    def summary_tree_bootstrap_daily(self, target_date=None, namespace: str = "general") -> Optional[int]:
        """Create a daily (L0) summary entry for the given date."""
        from .summary_tree import SummaryTreeEngine
        engine = SummaryTreeEngine(self)
        return engine.bootstrap_daily(target_date=target_date, namespace=namespace)

    def summary_tree_cascade(self, namespace: str = "general") -> dict:
        """Check all levels for cascade opportunities. Returns {level: count}."""
        from .summary_tree import SummaryTreeEngine
        engine = SummaryTreeEngine(self)
        return engine.cascade(namespace=namespace)

    def summary_tree_generate(self, summary_id: int) -> bool:
        """Generate LLM summary for a pending entry. Requires LLM callable."""
        from .summary_tree import SummaryTreeEngine
        engine = SummaryTreeEngine(self)
        return engine.generate_summary(summary_id)

    def get_summaries(self, level: Optional[int] = None,
                      namespace: str = "general", limit: int = 50) -> list[dict]:
        """Fetch summary entries."""
        from .summary_tree import SummaryTreeEngine
        engine = SummaryTreeEngine(self)
        return engine.get_summaries(level=level, namespace=namespace, limit=limit)

    def get_latest_summary(self, level: int = 1,
                           namespace: str = "general") -> Optional[dict]:
        """Get the most recent completed summary at the given level."""
        from .summary_tree import SummaryTreeEngine
        engine = SummaryTreeEngine(self)
        return engine.get_latest_summary(level=level, namespace=namespace)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _execute_write(self, fn):
        if not self._conn:
            raise RuntimeError("EntityStore not initialized")
        with self._lock:
            self._conn.execute("BEGIN IMMEDIATE")
            try:
                result = fn(self._conn)
                self._conn.commit()
                return result
            except BaseException:
                try:
                    self._conn.rollback()
                except Exception:
                    pass
                raise
