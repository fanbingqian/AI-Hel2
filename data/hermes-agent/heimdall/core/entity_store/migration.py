"""Database migration logic for Knowledge Ring schema evolution (V2.2).

Handles:
  - Old heimdall_entities → kr_entities (9 types → 5 types with type_detail)
  - Old kr_entities (type column) → new kr_entities (types JSON array)
  - Old kr_profiles (content + content_encrypted) → new (content_encrypted only)
  - Old kr_aliases → new kr_aliases (add last_seen, remove recency_score)
  - Old kr_originals → new kr_originals (add compressed_content, archive_status)
"""

import json
import logging
import uuid

logger = logging.getLogger(__name__)


class MigrationRunner:
    """Data migration runner for schema evolution."""

    def __init__(self, conn, lock):
        self._conn = conn
        self._lock = lock

    def migrate_all(self) -> dict:
        """Run all pending migrations. Returns summary dict."""
        stats = {}

        stats["v1_entities"] = self._migrate_v1_entities()
        stats["types_column"] = self._migrate_types_column()
        stats["profiles_encrypted"] = self._migrate_profiles_encrypted()
        stats["aliases_revised"] = self._migrate_aliases()

        return stats

    def _migrate_v1_entities(self) -> int:
        """Migrate old heimdall_entities → kr_entities (one-time).

        Checks if kr_entities is empty AND heimdall_entities has data.
        Safe to re-run — skips if kr_entities already has records.
        """
        if not self._conn:
            return 0

        new_count = self._conn.execute(
            "SELECT COUNT(*) as cnt FROM kr_entities"
        ).fetchone()["cnt"]
        if new_count > 0:
            return 0

        old_count = self._conn.execute(
            "SELECT COUNT(*) as cnt FROM heimdall_entities WHERE status = 'active'"
        ).fetchone()["cnt"]
        if old_count == 0:
            return 0

        logger.info("Migrating %d old entities to Knowledge Ring schema...", old_count)

        def _do(conn):
            from .schema import ENTITY_TYPE_MIGRATION_MAP
            old_rows = conn.execute(
                "SELECT * FROM heimdall_entities WHERE status = 'active'"
            ).fetchall()

            migrated = 0
            for row in old_rows:
                old_type = row["entity_type"]
                new_type, type_detail = ENTITY_TYPE_MIGRATION_MAP.get(
                    old_type, ("concept", old_type)
                )

                attrs = {}
                if row["attributes_json"]:
                    try:
                        attrs = json.loads(row["attributes_json"])
                    except (json.JSONDecodeError, TypeError):
                        pass

                props = {
                    "old_entity_id": row["entity_id"],
                    "occurrence_count": row["occurrence_count"],
                    "first_seen_at": row["first_seen_at"],
                }
                if isinstance(attrs, dict):
                    props.update(attrs)

                entity_id = uuid.uuid4().hex
                conn.execute(
                    "INSERT INTO kr_entities "
                    "(entity_id, name, types, type_detail, properties, confidence, "
                    "created_at, updated_at) "
                    "VALUES (?, ?, ?, ?, ?, ?, "
                    "COALESCE(datetime(?, 'unixepoch'), CURRENT_TIMESTAMP), "
                    "COALESCE(datetime(?, 'unixepoch'), CURRENT_TIMESTAMP))",
                    (entity_id, row["display_name"],
                     json.dumps([new_type], ensure_ascii=False),
                     type_detail,
                     json.dumps(props, ensure_ascii=False),
                     row["confidence"],
                     row["first_seen_at"], row["last_seen_at"]),
                )
                migrated += 1

            logger.info("V1 entity migration complete: %d entities migrated", migrated)
            return migrated

        return self._execute_write(_do)

    def _migrate_types_column(self) -> int:
        """Migrate kr_entities with old 'type' column to new 'types' JSON array.

        Handles: (a) DB has 'type' but not 'types' → add column + migrate,
        (b) both exist but rows have stale data → migrate.
        """
        if not self._conn:
            return 0

        cols = self._conn.execute("PRAGMA table_info(kr_entities)").fetchall()
        col_names = {c["name"] for c in cols}

        # Case (a): 'types' column missing entirely — add it
        if "types" not in col_names:
            self._conn.execute(
                "ALTER TABLE kr_entities ADD COLUMN types TEXT DEFAULT '[\"concept\"]'"
            )
            col_names.add("types")
            logger.info("Added missing 'types' column to kr_entities")

        # Nothing to migrate from
        if "type" not in col_names:
            return 0

        # Find rows where types is still default/empty but type has a value
        rows = self._conn.execute(
            "SELECT entity_id, type, types FROM kr_entities "
            "WHERE (types IS NULL OR types = '' OR types = '[]' OR types = '[\"concept\"]')"
        ).fetchall()

        migrated = 0
        for row in rows:
            old_type = row["type"]
            if old_type:
                new_types = json.dumps([old_type], ensure_ascii=False)
                self._conn.execute(
                    "UPDATE kr_entities SET types = ?, updated_at = CURRENT_TIMESTAMP "
                    "WHERE entity_id = ?",
                    (new_types, row["entity_id"]),
                )
                migrated += 1

        if migrated:
            logger.info("Types column migration: %d rows updated", migrated)
        return migrated

    def _migrate_profiles_encrypted(self) -> int:
        """Migrate kr_profiles: encrypt any remaining plaintext content.

        Checks for rows with content_encrypted IS NULL and content has text.
        """
        if not self._conn:
            return 0

        cols = self._conn.execute("PRAGMA table_info(kr_profiles)").fetchall()
        col_names = {c["name"] for c in cols}

        if "content_encrypted" not in col_names:
            return 0

        has_content = "content" in col_names

        if has_content:
            rows = self._conn.execute(
                "SELECT profile_id, content FROM kr_profiles "
                "WHERE content_encrypted IS NULL AND content IS NOT NULL AND content != ''"
            ).fetchall()
        else:
            rows = self._conn.execute(
                "SELECT profile_id FROM kr_profiles "
                "WHERE content_encrypted IS NULL"
            ).fetchall()

        migrated = 0
        from .crypto import device_encrypt

        for row in rows:
            content = row["content"] if has_content else ""
            if not content:
                continue
            encrypted = device_encrypt(content)
            self._conn.execute(
                "UPDATE kr_profiles SET content_encrypted = ?, updated_at = CURRENT_TIMESTAMP "
                "WHERE profile_id = ?",
                (encrypted, row["profile_id"]),
            )
            migrated += 1

        if migrated:
            logger.info("Profile encryption migration: %d rows encrypted", migrated)
        return migrated

    def _migrate_aliases(self) -> int:
        """Ensure kr_aliases has last_seen and confidence columns (V2.2 add)."""
        if not self._conn:
            return 0

        cols = self._conn.execute("PRAGMA table_info(kr_aliases)").fetchall()
        col_names = {c["name"] for c in cols}

        changes = 0

        if "last_seen" not in col_names:
            try:
                self._conn.execute(
                    "ALTER TABLE kr_aliases ADD COLUMN last_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP"
                )
                changes += 1
            except Exception:
                pass

        if "confidence" not in col_names:
            try:
                self._conn.execute(
                    "ALTER TABLE kr_aliases ADD COLUMN confidence REAL DEFAULT 0.5"
                )
                changes += 1
            except Exception:
                pass

        if changes:
            logger.info("Aliases migration: %d columns added", changes)

        return changes

    def _execute_write(self, fn):
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
