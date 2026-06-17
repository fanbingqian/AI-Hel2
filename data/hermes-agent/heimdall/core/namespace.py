"""Namespace soft isolation — per-namespace configuration management.

Soft isolation means all entities share a global knowledge tree, but each
namespace has its own:
  - Retrieval weight preferences (vector vs keyword vs graph vs temporal)
  - Entity type weights for ranking
  - Context injection rules (what to include in agent context)
  - Display/visibility preferences

The global tree is the same; namespace config acts as a filter/view.
"""

from __future__ import annotations

import json
import logging
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Optional

logger = logging.getLogger(__name__)

DEFAULT_RETRIEVER_WEIGHTS = {
    "vector": 0.35,
    "keyword": 0.25,
    "graph": 0.20,
    "temporal": 0.20,
}

DEFAULT_CONTEXT_INJECTION = {
    "knowledge": True,
    "summaries": True,
    "code_symbols": True,
}


@dataclass
class NamespaceConfig:
    id: str
    name: str
    description: str
    retriever_weights: dict[str, float]
    entity_type_weights: dict[str, float]
    context_injection: dict[str, bool]
    created_at: str
    updated_at: str


class NamespaceConfigManager:
    """Manage per-namespace configuration with persistence to kr_namespaces."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def get_config(self, namespace_name: str) -> Optional[dict]:
        """Get the full configuration for a namespace."""
        conn = self._store._conn
        row = conn.execute(
            "SELECT * FROM kr_namespaces WHERE name = ?", (namespace_name,)
        ).fetchone()
        if not row:
            return None

        config = {}
        if row["config"]:
            try:
                config = json.loads(row["config"])
            except (json.JSONDecodeError, TypeError):
                pass

        return {
            "id": row["id"],
            "name": row["name"],
            "description": row["description"] or "",
            "config": config,
            "entity_count": self._count_entities(namespace_name),
            "created_at": row["created_at"],
            "updated_at": row["updated_at"],
        }

    def list_configs(self) -> list[dict]:
        """List all namespace configurations with entity counts."""
        conn = self._store._conn
        rows = conn.execute(
            "SELECT * FROM kr_namespaces ORDER BY name"
        ).fetchall()

        result: list[dict] = []
        for row in rows:
            config = {}
            if row["config"]:
                try:
                    config = json.loads(row["config"])
                except Exception:
                    pass

            result.append({
                "id": row["id"],
                "name": row["name"],
                "description": row["description"] or "",
                "config": config,
                "entity_count": self._count_entities(row["name"]),
                "created_at": row["created_at"],
                "updated_at": row["updated_at"],
            })
        return result

    def create_or_update(
        self,
        namespace_name: str,
        description: str = "",
        retriever_weights: Optional[dict[str, float]] = None,
        entity_type_weights: Optional[dict[str, float]] = None,
        context_injection: Optional[dict[str, bool]] = None,
    ) -> dict:
        """Create or update a namespace configuration."""
        conn = self._store._conn
        now = time.strftime("%Y-%m-%dT%H:%M:%S")

        config = {
            "retriever_weights": retriever_weights or DEFAULT_RETRIEVER_WEIGHTS,
            "entity_type_weights": entity_type_weights or {},
            "context_injection": context_injection or DEFAULT_CONTEXT_INJECTION,
        }

        existing = conn.execute(
            "SELECT id FROM kr_namespaces WHERE name = ?", (namespace_name,)
        ).fetchone()

        if existing:
            conn.execute(
                """UPDATE kr_namespaces
                   SET description = ?, config = ?, updated_at = ?
                   WHERE name = ?""",
                (description, json.dumps(config, ensure_ascii=False), now, namespace_name),
            )
        else:
            ns_id = str(uuid.uuid5(
                uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
                namespace_name,
            ))
            conn.execute(
                """INSERT INTO kr_namespaces (id, name, description, config, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?)""",
                (ns_id, namespace_name, description, json.dumps(config, ensure_ascii=False), now, now),
            )

        conn.commit()
        return self.get_config(namespace_name) or {}

    def delete(self, namespace_name: str) -> bool:
        """Delete a namespace configuration (does NOT delete entities)."""
        conn = self._store._conn
        conn.execute("DELETE FROM kr_namespaces WHERE name = ?", (namespace_name,))
        conn.commit()
        return True

    def _count_entities(self, namespace_name: str) -> int:
        conn = self._store._conn
        row = conn.execute(
            "SELECT COUNT(*) FROM kr_entities WHERE namespace = ?",
            (namespace_name,),
        ).fetchone()
        return row[0] if row else 0
