"""HEIMDALL Media Reference Index — lightweight media file tracking.

Indexes media files (images, videos, documents, links) referenced in
conversations. Media is linked to entities via heimdall_media_refs table.
"""

from __future__ import annotations

import time
import uuid
from pathlib import Path
from typing import Optional

from heimdall.core.entity_store import EntityStore


class MediaRefIndex:
    """Lightweight index of media files referenced in conversations."""

    def __init__(self, store: EntityStore):
        self.store = store

    def add_ref(
        self,
        entity_id: Optional[str],
        media_type: str,
        uri: str,
        description: str = "",
    ) -> str:
        """Add a media reference linked to an entity. Returns ref_id."""
        ref_id = uuid.uuid4().hex

        def _do(conn):
            conn.execute(
                "INSERT INTO heimdall_media_refs (ref_id, entity_id, media_type, uri, description, created_at) "
                "VALUES (?, ?, ?, ?, ?, ?)",
                (ref_id, entity_id, media_type, uri, description, time.time()),
            )

        self.store._execute_write(_do)
        return ref_id

    def get_refs_for_entity(self, entity_id: str) -> list[dict]:
        """Get all media references for an entity."""
        if not self.store._conn:
            return []
        rows = self.store._conn.execute(
            "SELECT * FROM heimdall_media_refs WHERE entity_id = ? ORDER BY created_at DESC",
            (entity_id,),
        ).fetchall()
        return [dict(r) for r in rows]

    def get_recent_refs(self, limit: int = 20) -> list[dict]:
        """Get recently added media references."""
        if not self.store._conn:
            return []
        rows = self.store._conn.execute(
            "SELECT * FROM heimdall_media_refs ORDER BY created_at DESC LIMIT ?",
            (limit,),
        ).fetchall()
        return [dict(r) for r in rows]
