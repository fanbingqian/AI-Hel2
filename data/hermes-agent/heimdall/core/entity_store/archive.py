"""Original text archive with TTL lifecycle and zlib compression (V2.2).

Archive strategy:
  - 0-7 days:   full content, archive_status='local'
  - 7-30 days:  truncated to 512 chars, archive_status='summary'
  - 30-90 days: zlib compressed to compressed_content BLOB, archive_status='compressed'
  - 90+ days:   content + compressed_content cleared, archive_status='deleted'
"""

import logging
import zlib
from datetime import datetime, timedelta
from typing import Optional

logger = logging.getLogger(__name__)


class ArchiveManager:
    """Originals TTL archive manager with zlib compression."""

    LOCAL_DAYS = 7
    SUMMARY_DAYS = 30
    ARCHIVE_DAYS = 90

    def __init__(self, conn):
        self._conn = conn

    def run_daily_archive(self, today: Optional[datetime] = None) -> dict:
        """Execute daily archive task. Returns stats dict."""
        today = today or datetime.now()
        stats = {"summarized": 0, "compressed": 0, "deleted": 0}

        # 7→30 days: truncate to summary
        summary_cutoff = today - timedelta(days=self.LOCAL_DAYS)
        cur = self._conn.execute(
            "UPDATE kr_originals SET "
            "content = substr(content, 1, 512), "
            "archive_status = 'summary', "
            "archived_at = ? "
            "WHERE archive_status = 'local' AND created_at < ?",
            (today.isoformat(), summary_cutoff.isoformat()),
        )
        stats["summarized"] = cur.rowcount

        # 30→90 days: zlib compress
        archive_cutoff = today - timedelta(days=self.SUMMARY_DAYS)
        rows = self._conn.execute(
            "SELECT original_id, content FROM kr_originals "
            "WHERE archive_status = 'summary' AND created_at < ?",
            (archive_cutoff.isoformat(),),
        ).fetchall()

        for row in rows:
            if row["content"]:
                try:
                    compressed = zlib.compress(row["content"].encode(), level=6)
                    self._conn.execute(
                        "UPDATE kr_originals SET "
                        "content = '', "
                        "compressed_content = ?, "
                        "archive_status = 'compressed', "
                        "archived_at = ? "
                        "WHERE original_id = ?",
                        (compressed, today.isoformat(), row["original_id"]),
                    )
                    stats["compressed"] += 1
                except Exception:
                    logger.debug("Failed to compress original %s", row["original_id"])

        # 90+ days: delete content
        delete_cutoff = today - timedelta(days=self.ARCHIVE_DAYS)
        cur = self._conn.execute(
            "UPDATE kr_originals SET "
            "content = '', "
            "compressed_content = NULL, "
            "archive_status = 'deleted', "
            "archived_at = ? "
            "WHERE archive_status = 'compressed' AND created_at < ?",
            (today.isoformat(), delete_cutoff.isoformat()),
        )
        stats["deleted"] = cur.rowcount

        if any(stats.values()):
            logger.info("Archive run complete: %s", stats)
        return stats

    def retrieve_original(self, original_id: int) -> Optional[str]:
        """Retrieve original text, handling all archive states."""
        row = self._conn.execute(
            "SELECT * FROM kr_originals WHERE original_id = ?",
            (original_id,),
        ).fetchone()
        if not row:
            return None

        status = row["archive_status"]

        if status == "local":
            return row["content"]
        elif status == "summary":
            return (row["content"] or "") + "\n\n[已归档为摘要]"
        elif status == "compressed":
            if row["compressed_content"]:
                try:
                    return zlib.decompress(row["compressed_content"]).decode()
                except Exception:
                    return "[压缩内容损坏]"
            return "[压缩内容缺失]"
        elif status == "deleted":
            return "[内容已过期删除]"

        return None

    def get_archive_stats(self) -> dict:
        """Get counts by archive status."""
        rows = self._conn.execute(
            "SELECT archive_status, COUNT(*) as cnt FROM kr_originals GROUP BY archive_status"
        ).fetchall()
        return {r["archive_status"]: r["cnt"] for r in rows}
