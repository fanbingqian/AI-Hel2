"""Entity store CRUD operations (V2.2).

All database reads/writes centralized here. Delegates to crypto for
profile encryption and archive for original text lifecycle.
"""

import json
import logging
import time
import uuid
from datetime import date, datetime, timedelta
from typing import Any, Optional

from .crypto import device_decrypt, device_encrypt
from .schema import (
    MEMORY_EMBEDDING_BYTES,
    VALID_ENTITY_TYPES,
    VALID_EVENT_LOG_TYPES,
    VALID_PROFILE_TYPES,
    VALID_RELATION_DIRECTIONS,
    VALID_RELATION_TYPES,
)

logger = logging.getLogger(__name__)


def _sanitize_fts_query(query: str) -> str:
    """Sanitize user query for FTS5, escaping special characters."""
    import re
    cleaned = re.sub(r'[^\w\s一-鿿]', '', query.strip())
    if not cleaned:
        return '""'
    terms = cleaned.split()
    return " AND ".join(f'"{t}"' for t in terms[:10])


class EntityStoreOps:
    """Knowledge Ring entity CRUD operations."""

    def __init__(self, conn, lock):
        self._conn = conn
        self._lock = lock

    @staticmethod
    def _generate_id(name: str) -> str:
        namespace = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
        return str(uuid.uuid5(namespace, name.lower().strip()))

    @staticmethod
    def _compute_recency(days: int, lambda_rate: float = 0.05) -> float:
        import math
        return math.exp(-lambda_rate * max(days, 0))

    # -------------------------------------------------------------------
    # Entity CRUD
    # -------------------------------------------------------------------

    def upsert_entity(
        self,
        name: str,
        types: Optional[list] = None,
        type_detail: str = "",
        properties: Optional[dict] = None,
        confidence: float = 0.5,
        domains: Optional[list] = None,
        tags: Optional[list] = None,
        hrr_vector: Optional[bytes] = None,
        memory_embedding: Optional[bytes] = None,
        session_id: str = "",
        namespace: str = "general",
    ) -> str:
        """Insert or update an entity (V2.3: + namespace)."""
        types = types or ["concept"]
        for t in types:
            if t not in VALID_ENTITY_TYPES:
                raise ValueError(f"Invalid entity type: {t}")

        if memory_embedding is not None and len(memory_embedding) != MEMORY_EMBEDDING_BYTES:
            raise ValueError(
                f"memory_embedding size mismatch: expected {MEMORY_EMBEDDING_BYTES}, "
                f"got {len(memory_embedding)}"
            )

        entity_id = self._generate_id(name)
        types_json = json.dumps(types, ensure_ascii=False)
        domains_json = json.dumps(domains or [], ensure_ascii=False)
        properties_json = json.dumps(properties or {}, ensure_ascii=False)
        tags_json = json.dumps(tags or [], ensure_ascii=False)

        def _do(conn):
            existing = conn.execute(
                "SELECT entity_id FROM kr_entities WHERE entity_id = ?",
                (entity_id,),
            ).fetchone()

            if existing:
                conn.execute("""
                    UPDATE kr_entities SET
                        types = ?, type_detail = ?, domains = ?,
                        properties = ?, tags = ?,
                        confidence = MAX(confidence, ?),
                        hrr_vector = COALESCE(?, hrr_vector),
                        memory_embedding = COALESCE(?, memory_embedding),
                        updated_at = CURRENT_TIMESTAMP
                    WHERE entity_id = ?
                """, (types_json, type_detail, domains_json, properties_json,
                      tags_json, confidence, hrr_vector, memory_embedding, entity_id))
                self._log_event(conn, "entity_updated", entity_id=entity_id,
                               session_id=session_id,
                               description=f"Updated {types[0]}: {name}")
            else:
                conn.execute("""
                    INSERT INTO kr_entities
                        (entity_id, name, types, type_detail, domains,
                         properties, tags, confidence, hrr_vector, memory_embedding, namespace)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """, (entity_id, name, types_json, type_detail, domains_json,
                      properties_json, tags_json, confidence, hrr_vector, memory_embedding, namespace))
                self._log_event(conn, "entity_created", entity_id=entity_id,
                               session_id=session_id,
                               description=f"Created {types[0]}: {name}")

            return entity_id

        return self._execute_write(_do)

    def get_entity(self, entity_id: str) -> Optional[dict]:
        if not self._conn:
            return None
        row = self._conn.execute(
            "SELECT * FROM kr_entities WHERE entity_id = ?", (entity_id,)
        ).fetchone()
        if not row:
            return None
        return self._parse_entity_row(dict(row))

    def get_entity_by_name(self, name: str) -> Optional[dict]:
        entity_id = self._generate_id(name)
        return self.get_entity(entity_id)

    def list_entities(
        self, entity_type: Optional[str] = None, domain: Optional[str] = None,
        limit: int = 50, offset: int = 0,
    ) -> list[dict]:
        if not self._conn:
            return []
        clauses = []
        params: list[Any] = []
        if entity_type:
            clauses.append("types LIKE ?")
            params.append(f'%"{entity_type}"%')
        if domain:
            clauses.append("domains LIKE ?")
            params.append(f'%"{domain}"%')
        where = ("WHERE " + " AND ".join(clauses)) if clauses else ""
        rows = self._conn.execute(
            f"SELECT * FROM kr_entities {where} "
            "ORDER BY updated_at DESC LIMIT ? OFFSET ?",
            (*params, limit, offset),
        ).fetchall()
        return [self._parse_entity_row(dict(r)) for r in rows]

    def search_entities(self, query: str, limit: int = 10) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        sanitized = _sanitize_fts_query(query)
        try:
            rows = self._conn.execute(
                "SELECT e.* FROM kr_entities e "
                "JOIN kr_entities_fts fts ON e.rowid = fts.rowid "
                "WHERE kr_entities_fts MATCH ? ORDER BY rank LIMIT ?",
                (sanitized, limit),
            ).fetchall()
        except Exception:
            return []
        return [self._parse_entity_row(dict(r)) for r in rows]

    def update_entity(self, entity_id: str, **fields) -> bool:
        allowed = {"name", "type_detail", "domains", "properties", "tags",
                    "confidence", "source_ref", "timeline", "types"}
        updates = {k: v for k, v in fields.items() if k in allowed}
        if not updates:
            return False

        def _do(conn):
            old = conn.execute(
                "SELECT * FROM kr_entities WHERE entity_id = ?", (entity_id,)
            ).fetchone()
            if not old:
                return False
            set_parts = []
            params = []
            for k, v in updates.items():
                val = json.dumps(v, ensure_ascii=False) if isinstance(v, (dict, list)) else v
                set_parts.append(f"{k} = ?")
                params.append(val)
            params.append(entity_id)
            conn.execute(
                f"UPDATE kr_entities SET {', '.join(set_parts)}, "
                "updated_at = CURRENT_TIMESTAMP WHERE entity_id = ?",
                params,
            )
            for k, v in updates.items():
                old_val = str(old[k]) if k in old.keys() else ""
                new_val = json.dumps(v, ensure_ascii=False) if isinstance(v, (dict, list)) else str(v)
                self._log_event(conn, "field_edited", entity_id=entity_id,
                               description=f"Changed {k}", old_value=old_val,
                               new_value=new_val, source="user_correction")
            return True

        return self._execute_write(_do)

    def get_entity_count(self) -> int:
        if not self._conn:
            return 0
        row = self._conn.execute("SELECT COUNT(*) as cnt FROM kr_entities").fetchone()
        return row["cnt"] if row else 0

    # -------------------------------------------------------------------
    # Namespace-aware queries (V2.3)
    # -------------------------------------------------------------------

    def count_by_namespace(self, namespace: str) -> int:
        if not self._conn:
            return 0
        row = self._conn.execute(
            "SELECT COUNT(*) as cnt FROM kr_entities WHERE namespace = ?",
            (namespace,),
        ).fetchone()
        return row["cnt"] if row else 0

    def count_all(self) -> int:
        return self.get_entity_count()

    def list_namespaces(self) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT namespace, COUNT(*) as cnt FROM kr_entities "
            "GROUP BY namespace ORDER BY cnt DESC"
        ).fetchall()
        return [dict(r) for r in rows]

    def list_all(self, limit: int = 10000) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT entity_id, name, types, namespace, confidence, "
            "updated_at FROM kr_entities ORDER BY updated_at DESC LIMIT ?",
            (limit,),
        ).fetchall()
        return [self._parse_entity_row(dict(r)) for r in rows]

    def list_by_namespace(self, namespace: str, limit: int = 5000) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT entity_id, name, types, namespace, confidence, "
            "updated_at FROM kr_entities WHERE namespace = ? "
            "ORDER BY updated_at DESC LIMIT ?",
            (namespace, limit),
        ).fetchall()
        return [self._parse_entity_row(dict(r)) for r in rows]

    def _parse_entity_row(self, row: dict) -> dict:
        for field in ("types", "domains", "properties", "tags", "timeline"):
            if field in row and isinstance(row[field], str):
                try:
                    row[field] = json.loads(row[field])
                except (json.JSONDecodeError, TypeError):
                    row[field] = [] if field in ("types", "domains", "tags", "timeline") else {}
        return row

    # -------------------------------------------------------------------
    # Aliases (V2.2: recency_score computed at query time)
    # -------------------------------------------------------------------

    def add_alias(
        self, entity_id: str, name: str,
        context: str = "", confidence: float = 0.5, session_id: str = "",
    ) -> int:
        def _do(conn):
            existing = conn.execute(
                "SELECT alias_id FROM kr_aliases WHERE entity_id = ? AND name = ?",
                (entity_id, name),
            ).fetchone()
            if existing:
                conn.execute(
                    "UPDATE kr_aliases SET last_seen = CURRENT_TIMESTAMP, "
                    "confidence = MAX(confidence, ?) WHERE alias_id = ?",
                    (confidence, existing["alias_id"]),
                )
                return existing["alias_id"]
            cur = conn.execute(
                "INSERT INTO kr_aliases (entity_id, name, context, confidence) "
                "VALUES (?, ?, ?, ?)",
                (entity_id, name, context, confidence),
            )
            return cur.lastrowid

        return self._execute_write(_do)

    def get_aliases(self, entity_id: str) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM kr_aliases WHERE entity_id = ? ORDER BY last_seen DESC",
            (entity_id,),
        ).fetchall()
        result = []
        now = datetime.now()
        for row in rows:
            d = dict(row)
            if d.get("last_seen"):
                try:
                    last_seen = datetime.fromisoformat(str(d["last_seen"]))
                    days = (now - last_seen).days
                except (ValueError, TypeError):
                    days = 30
                d["recency_score"] = self._compute_recency(days)
            else:
                d["recency_score"] = 0.0
            result.append(d)
        return result

    def search_aliases(self, query: str, limit: int = 10) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        sanitized = _sanitize_fts_query(query)
        try:
            rows = self._conn.execute(
                "SELECT a.*, e.name as entity_name, e.types as entity_types "
                "FROM kr_aliases a "
                "JOIN kr_aliases_fts fts ON a.rowid = fts.rowid "
                "JOIN kr_entities e ON a.entity_id = e.entity_id "
                "WHERE kr_aliases_fts MATCH ? ORDER BY rank LIMIT ?",
                (sanitized, limit),
            ).fetchall()
        except Exception:
            return []
        return [dict(r) for r in rows]

    # -------------------------------------------------------------------
    # Relations
    # -------------------------------------------------------------------

    def add_relation(
        self, source_id: str, target_id: str, rel_type: str,
        confidence: float = 0.5, direction: str = "bidirectional",
        source_text: str = "", session_id: str = "",
        namespace: str = "general",
    ) -> int:
        if rel_type not in VALID_RELATION_TYPES:
            raise ValueError(f"Invalid relation type: {rel_type}")
        if direction not in VALID_RELATION_DIRECTIONS:
            raise ValueError(f"Invalid direction: {direction}")

        def _do(conn):
            existing = conn.execute(
                "SELECT relation_id FROM kr_relations "
                "WHERE ((source_id = ? AND target_id = ?) "
                "OR (source_id = ? AND target_id = ? AND direction = 'bidirectional')) "
                "AND type = ?",
                (source_id, target_id, target_id, source_id, rel_type),
            ).fetchone()
            if existing:
                conn.execute(
                    "UPDATE kr_relations SET confidence = MAX(confidence, ?), "
                    "source_text = CASE WHEN ? != '' THEN ? ELSE source_text END, "
                    "updated_at = CURRENT_TIMESTAMP WHERE relation_id = ?",
                    (confidence, source_text, source_text, existing["relation_id"]),
                )
                return existing["relation_id"]

            cur = conn.execute(
                "INSERT INTO kr_relations "
                "(source_id, target_id, type, confidence, direction, source_text, namespace) "
                "VALUES (?, ?, ?, ?, ?, ?, ?)",
                (source_id, target_id, rel_type, confidence, direction, source_text, namespace),
            )
            rid = cur.lastrowid
            self._log_event(conn, "relation_added", entity_id=source_id,
                           relation_id=rid,
                           description=f"{rel_type}: {source_id} -> {target_id}",
                           session_id=session_id)
            return rid

        return self._execute_write(_do)

    def get_relations(self, entity_id: str) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT r.*, e1.name as source_name, e1.types as source_types, "
            "e2.name as target_name, e2.types as target_types "
            "FROM kr_relations r "
            "JOIN kr_entities e1 ON r.source_id = e1.entity_id "
            "JOIN kr_entities e2 ON r.target_id = e2.entity_id "
            "WHERE r.source_id = ? OR r.target_id = ? "
            "ORDER BY r.confidence DESC",
            (entity_id, entity_id),
        ).fetchall()
        return [dict(r) for r in rows]

    def find_relation_path(self, source_id: str, target_id: str, max_depth: int = 2) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM kr_relations "
            "WHERE (source_id = ? AND target_id = ?) OR (source_id = ? AND target_id = ?)",
            (source_id, target_id, target_id, source_id),
        ).fetchall()
        return [dict(r) for r in rows]

    def search_relations(self, query: str, limit: int = 10) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        sanitized = _sanitize_fts_query(query)
        try:
            rows = self._conn.execute(
                "SELECT r.*, e1.name as source_name, e2.name as target_name "
                "FROM kr_relations r "
                "JOIN kr_relations_fts fts ON r.rowid = fts.rowid "
                "JOIN kr_entities e1 ON r.source_id = e1.entity_id "
                "JOIN kr_entities e2 ON r.target_id = e2.entity_id "
                "WHERE kr_relations_fts MATCH ? ORDER BY rank LIMIT ?",
                (sanitized, limit),
            ).fetchall()
        except Exception:
            return []
        return [dict(r) for r in rows]

    def get_relation_count(self) -> int:
        if not self._conn:
            return 0
        row = self._conn.execute("SELECT COUNT(*) as cnt FROM kr_relations").fetchone()
        return row["cnt"] if row else 0

    # -------------------------------------------------------------------
    # Originals (V2.2: with archive lifecycle)
    # -------------------------------------------------------------------

    def add_original(
        self, content: str, source_type: str = "dialog",
        metadata: Optional[dict] = None, local_ttl: int = 7, session_id: str = "",
        namespace: str = "general",
    ) -> int:
        metadata_json = json.dumps(metadata or {}, ensure_ascii=False)

        def _do(conn):
            cur = conn.execute(
                "INSERT INTO kr_originals "
                "(source_type, content, metadata, local_ttl, archive_status, namespace) "
                "VALUES (?, ?, ?, ?, 'local', ?)",
                (source_type, content, metadata_json, local_ttl, namespace),
            )
            return cur.lastrowid

        return self._execute_write(_do)

    def search_originals(self, query: str, limit: int = 5) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        sanitized = _sanitize_fts_query(query)
        try:
            rows = self._conn.execute(
                "SELECT o.* FROM kr_originals o "
                "JOIN kr_originals_fts fts ON o.rowid = fts.rowid "
                "WHERE kr_originals_fts MATCH ? ORDER BY rank LIMIT ?",
                (sanitized, limit),
            ).fetchall()
        except Exception:
            return []
        return [dict(r) for r in rows]

    def get_original(self, original_id: int) -> Optional[dict]:
        if not self._conn:
            return None
        row = self._conn.execute(
            "SELECT * FROM kr_originals WHERE original_id = ?", (original_id,)
        ).fetchone()
        return dict(row) if row else None

    # -------------------------------------------------------------------
    # Profiles (V2.2: encrypted only, no plaintext)
    # -------------------------------------------------------------------

    def add_profile(
        self, profile_type: str, content: str,
        trigger_context: str = "", confidence: float = 0.5,
        tag: str = "ai_only", ttl_days: Optional[int] = None, session_id: str = "",
    ) -> int:
        if profile_type not in VALID_PROFILE_TYPES:
            raise ValueError(f"Invalid profile type: {profile_type}")

        encrypted = device_encrypt(content)
        expires_at = None
        if ttl_days:
            cur = self._conn.execute(
                "SELECT datetime('now', '+? days')", (ttl_days,)
            )
            row = cur.fetchone()
            if row:
                expires_at = row[0]

        def _do(conn):
            cur = conn.execute(
                "INSERT INTO kr_profiles "
                "(type, content_encrypted, trigger_context, confidence, tag, "
                "ttl_days, expires_at) "
                "VALUES (?, ?, ?, ?, ?, ?, ?)",
                (profile_type, encrypted, trigger_context, confidence, tag,
                 ttl_days, expires_at),
            )
            pid = cur.lastrowid
            self._log_event(conn, "profile_created", session_id=session_id,
                           description=f"{profile_type}: {content[:50]}...")
            return pid

        return self._execute_write(_do)

    def get_profile(self, profile_id: int) -> Optional[dict]:
        if not self._conn:
            return None
        row = self._conn.execute(
            "SELECT * FROM kr_profiles WHERE profile_id = ?", (profile_id,)
        ).fetchone()
        if not row:
            return None
        result = dict(row)
        if result.get("content_encrypted"):
            try:
                result["content"] = device_decrypt(result["content_encrypted"])
            except Exception:
                result["content"] = "[解密失败]"
        return result

    def search_profiles(self, query: str, profile_type: Optional[str] = None,
                        limit: int = 10) -> list[dict]:
        if not self._conn or not query.strip():
            return []
        sanitized = _sanitize_fts_query(query)
        try:
            if profile_type:
                rows = self._conn.execute(
                    "SELECT p.* FROM kr_profiles p "
                    "JOIN kr_profiles_fts fts ON p.rowid = fts.rowid "
                    "WHERE kr_profiles_fts MATCH ? AND p.type = ? ORDER BY rank LIMIT ?",
                    (sanitized, profile_type, limit),
                ).fetchall()
            else:
                rows = self._conn.execute(
                    "SELECT p.* FROM kr_profiles p "
                    "JOIN kr_profiles_fts fts ON p.rowid = fts.rowid "
                    "WHERE kr_profiles_fts MATCH ? ORDER BY rank LIMIT ?",
                    (sanitized, limit),
                ).fetchall()
        except Exception:
            return []
        results = []
        for r in rows:
            d = dict(r)
            if d.get("content_encrypted"):
                try:
                    d["content"] = device_decrypt(d["content_encrypted"])
                except Exception:
                    d["content"] = "[解密失败]"
            results.append(d)
        return results

    def get_profiles_by_tag(self, tag: str = "ai_only") -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM kr_profiles WHERE tag = ? ORDER BY updated_at DESC", (tag,)
        ).fetchall()
        results = []
        for r in rows:
            d = dict(r)
            if d.get("content_encrypted"):
                try:
                    d["content"] = device_decrypt(d["content_encrypted"])
                except Exception:
                    d["content"] = "[解密失败]"
            results.append(d)
        return results

    # -------------------------------------------------------------------
    # Event Log
    # -------------------------------------------------------------------

    def _log_event(
        self, conn, event_type: str,
        entity_id: Optional[str] = None,
        relation_id: Optional[int] = None,
        domain_id: Optional[str] = None,
        description: str = "",
        old_value: str = "",
        new_value: str = "",
        source: str = "auto",
        session_id: str = "",
    ) -> None:
        conn.execute(
            "INSERT INTO kr_event_log "
            "(date, event_type, entity_id, relation_id, domain_id, source, "
            "description, old_value, new_value, session_id) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (date.today().isoformat(), event_type, entity_id, relation_id,
             domain_id, source, description, old_value, new_value, session_id),
        )

    def get_daily_events(self, target_date: Optional[str] = None) -> list[dict]:
        if not self._conn:
            return []
        d = target_date or date.today().isoformat()
        rows = self._conn.execute(
            "SELECT * FROM kr_event_log WHERE date = ? ORDER BY timestamp DESC", (d,)
        ).fetchall()
        return [dict(r) for r in rows]

    def get_event_date_range(self, start_date: str, end_date: str) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM kr_event_log WHERE date >= ? AND date <= ? "
            "ORDER BY date DESC, timestamp DESC",
            (start_date, end_date),
        ).fetchall()
        return [dict(r) for r in rows]

    def calculate_streak(self) -> int:
        if not self._conn:
            return 0
        rows = self._conn.execute(
            "SELECT DISTINCT date FROM kr_event_log ORDER BY date DESC LIMIT 100"
        ).fetchall()
        if not rows:
            return 0
        from datetime import timedelta
        today = date.today()
        streak = 0
        for i, row in enumerate(rows):
            expected = (today - timedelta(days=i)).isoformat()
            if row["date"] == expected:
                streak += 1
            elif i == 0 and row["date"] == (today - timedelta(days=1)).isoformat():
                streak += 1
            else:
                break
        return streak

    # -------------------------------------------------------------------
    # Domains
    # -------------------------------------------------------------------

    def register_domain(self, domain_name: str, first_entity_id: str = "",
                        status: str = "auto_created") -> None:
        def _do(conn):
            existing = conn.execute(
                "SELECT domain_name FROM kr_domain_first_seen WHERE domain_name = ?",
                (domain_name,),
            ).fetchone()
            if existing:
                conn.execute(
                    "UPDATE kr_domain_first_seen SET status = ? WHERE domain_name = ?",
                    (status, domain_name),
                )
            else:
                conn.execute(
                    "INSERT INTO kr_domain_first_seen "
                    "(domain_name, first_seen, first_entity_id, status) "
                    "VALUES (?, ?, ?, ?)",
                    (domain_name, date.today().isoformat(), first_entity_id, status),
                )

        self._execute_write(_do)

    def get_domains(self) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT * FROM kr_domain_first_seen ORDER BY first_seen DESC"
        ).fetchall()
        return [dict(r) for r in rows]

    def get_domain_stats(self) -> list[dict]:
        if not self._conn:
            return []
        rows = self._conn.execute(
            "SELECT types, COUNT(*) as cnt FROM kr_entities "
            "GROUP BY types ORDER BY cnt DESC"
        ).fetchall()
        return [dict(r) for r in rows]

    # -------------------------------------------------------------------
    # Multi-FTS search
    # -------------------------------------------------------------------

    def search_fts_multi(self, query: str, limit: int = 10) -> dict:
        return {
            "entities": self.search_entities(query, limit),
            "aliases": self.search_aliases(query, limit),
            "relations": self.search_relations(query, limit),
            "originals": self.search_originals(query, max(limit // 2, 1)),
            "profiles": self.search_profiles(query, limit),
        }

    # -------------------------------------------------------------------
    # Ring Graph data
    # -------------------------------------------------------------------

    def get_ring_graph_data(self) -> dict:
        if not self._conn:
            return {"nodes": [], "edges": []}
        # Detect whether schema uses 'types' (JSON array) or 'type' (scalar)
        cols = {c["name"] for c in self._conn.execute("PRAGMA table_info(kr_entities)").fetchall()}
        type_col = "types" if "types" in cols else "type"
        entities = self._conn.execute(
            f"SELECT entity_id, name, {type_col} AS types, type_detail, domains, "
            "namespace, confidence, properties, created_at FROM kr_entities ORDER BY name"
        ).fetchall()
        relations = self._conn.execute(
            "SELECT r.relation_id, r.source_id, r.target_id, r.type, r.confidence, "
            "r.direction, e1.name as source_name, e2.name as target_name "
            "FROM kr_relations r "
            "JOIN kr_entities e1 ON r.source_id = e1.entity_id "
            "JOIN kr_entities e2 ON r.target_id = e2.entity_id "
            "ORDER BY r.confidence DESC"
        ).fetchall()
        nodes = []
        for e in entities:
            d = dict(e)
            if isinstance(d.get("types"), str):
                try:
                    d["types"] = json.loads(d["types"])
                except (json.JSONDecodeError, TypeError):
                    d["types"] = ["concept"]
            # Backward compat: add 'type' field from first type
            types_list = d.get("types", ["concept"])
            d["type"] = types_list[0] if types_list else "concept"
            nodes.append(d)
        return {"nodes": nodes, "edges": [dict(r) for r in relations]}

    # -------------------------------------------------------------------
    # Knowledge Ring V2.3 — pending sync queue (unified retry)
    # -------------------------------------------------------------------

    def insert_pending_sync(self, payload_type: str, payload: dict,
                            namespace: str = "general") -> str:
        """Enqueue a failed sync operation for background retry.

        Unified entry point used by both the HTTP layer (api.py) and the
        Provider layer (HeimdallProvider) so that kr_pending_sync is no
        longer an HTTP-only concern.
        """
        pending_id = uuid.uuid4().hex
        self._conn.execute(
            "INSERT INTO kr_pending_sync (id, payload_type, payload_json, namespace) "
            "VALUES (?, ?, ?, ?)",
            (pending_id, payload_type, json.dumps(payload, ensure_ascii=False), namespace),
        )
        self._conn.commit()
        return pending_id

    def list_pending_sync(self, namespace: str = None, limit: int = 50) -> list[dict]:
        """List retryable pending records (retry_count < 5), ordered by next_retry_at."""
        sql = ("SELECT * FROM kr_pending_sync WHERE retry_count < 5")
        params = ()
        if namespace:
            sql += " AND namespace = ?"
            params = (namespace,)
        sql += " ORDER BY next_retry_at ASC LIMIT ?"
        params += (limit,)
        return [dict(r) for r in self._conn.execute(sql, params).fetchall()]

    def delete_pending_sync(self, pending_id: str) -> None:
        """Manually remove a pending record (user-confirmed discard)."""
        self._conn.execute("DELETE FROM kr_pending_sync WHERE id = ?", (pending_id,))
        self._conn.commit()

    def cleanup_dead_pending_sync(self, retention_days: int = 7) -> int:
        """Purge dead rows (retry_count >= 5) older than retention_days.

        Returns the number of deleted rows.
        """
        cutoff = (datetime.now() - timedelta(days=retention_days)).isoformat()
        result = self._conn.execute(
            "DELETE FROM kr_pending_sync WHERE retry_count >= 5 AND next_retry_at < ?",
            (cutoff,),
        )
        self._conn.commit()
        return result.rowcount

    def get_pending_failures(self, namespace: str = None) -> list[dict]:
        """List failed records (retry_count >= 5) for UI visibility."""
        sql = "SELECT * FROM kr_pending_sync WHERE retry_count >= 5"
        params = ()
        if namespace:
            sql += " AND namespace = ?"
            params = (namespace,)
        sql += " ORDER BY next_retry_at DESC"
        return [dict(r) for r in self._conn.execute(sql, params).fetchall()]

    # -------------------------------------------------------------------
    # Helpers
    # -------------------------------------------------------------------

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
