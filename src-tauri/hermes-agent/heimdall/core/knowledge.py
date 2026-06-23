"""HEIMDALL Knowledge Manager — KNOWLEDGE.md + knowledge entries + skill mastery.

The knowledge track is physically isolated from the memory track but
unified through the entity store's mapping layer.

Knowledge entries track:
  - domain / subdomain taxonomy
  - mastery_level: 了解 → 练习中 → 掌握 → 精通
  - access patterns for forgetting curve reminders

Mastery assessment is interactive, not formulaic:
  1. User self-assessment (slider: 不了解|了解|练习中|掌握|精通)
  2. AI-assisted suggestion (detects frequent discussion/application)
  3. Forgetting reminder (time-driven, >30 days without access)
"""

from __future__ import annotations

import logging
import time
from pathlib import Path
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

KNOWLEDGE_FILENAME = "KNOWLEDGE.md"

KNOWLEDGE_TEMPLATE = """# KNOWLEDGE

## 核心专业领域 [用户声明+AI辅助自评]
<!-- 格式: - 领域: 描述 [掌握度: 了解/练习中/掌握/精通] [来源: 工作/自学/兴趣] -->

## 知识来源图谱
<!-- 上传文档、查询历史、笔记等 -->

## 知识代理规则
- 掌握度为"了解"或"练习中"时主动提示不确定性
- 知识冲突时优先采用高可信度来源
- 超出掌握度"掌握"的领域时，建议云端深度检索
"""

MASTERY_LEVELS = ("不了解", "了解", "练习中", "掌握", "精通")


class KnowledgeManager:
    """Manages the knowledge track — KNOWLEDGE.md + structured entries.

    Knowledge is physically separate from memory but unified through
    the entity store. Skill mastery is tracked with interactive assessment,
    not pseudo-scientific formulas.
    """

    def __init__(self, heimdall_dir: Path, store: Optional[EntityStore] = None):
        self.heimdall_dir = heimdall_dir
        self.knowledge_path = heimdall_dir / KNOWLEDGE_FILENAME
        self.store = store
        self._snapshot: Optional[str] = None

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def load(self) -> str:
        """Load KNOWLEDGE.md. Creates from template if missing."""
        self.heimdall_dir.mkdir(parents=True, exist_ok=True)
        if not self.knowledge_path.exists():
            self.knowledge_path.write_text(KNOWLEDGE_TEMPLATE, encoding="utf-8")
        self._snapshot = self.knowledge_path.read_text(encoding="utf-8")
        return self._snapshot

    @property
    def snapshot(self) -> str:
        if self._snapshot is None:
            return self.load()
        return self._snapshot

    # ------------------------------------------------------------------
    # Knowledge entries
    # ------------------------------------------------------------------

    def add_entry(
        self,
        domain: str,
        title: str,
        content: str,
        mastery_level: str = "了解",
        confidence: float = 0.5,
        source_session_id: str = "",
        source_type: str = "",
        source_ref: str = "",
    ) -> Optional[str]:
        """Add a knowledge entry to the store.

        Returns entry_id or None if no store is configured.
        """
        if not self.store:
            return None
        return self.store.upsert_knowledge(
            domain=domain,
            title=title,
            content=content,
            mastery_level=mastery_level,
            confidence=confidence,
            source_session_id=source_session_id,
        )

    def search(self, query: str, domain: Optional[str] = None, limit: int = 10) -> list[dict]:
        """Search knowledge entries via FTS5."""
        if not self.store:
            return []
        results = self.store.search_knowledge(query, domain=domain, limit=limit)
        for r in results:
            self.store.touch_knowledge(r["entry_id"])
        return results

    def get_stale_entries(self, days: int = 30) -> list[dict]:
        """Get entries that need review (inactive for N days)."""
        if not self.store:
            return []
        return self.store.get_stale_knowledge(days=days)

    # ------------------------------------------------------------------
    # Skill mastery
    # ------------------------------------------------------------------

    def update_mastery(
        self, skill_name: str, mastery_level: str, parent_domain: str = ""
    ) -> Optional[str]:
        """Update or create a skill mastery record.

        mastery_level must be one of: 不了解, 了解, 练习中, 掌握, 精通
        """
        if mastery_level not in MASTERY_LEVELS:
            raise ValueError(f"Invalid mastery level: {mastery_level}. Must be one of {MASTERY_LEVELS}")
        if not self.store:
            return None
        return self.store.upsert_skill_mastery(
            skill_name=skill_name,
            parent_domain=parent_domain,
            mastery_level=mastery_level,
        )

    def get_skills_needing_review(self, days: int = 30) -> list[dict]:
        """Get skills that haven't been interacted with in N days."""
        if not self.store:
            return []
        return self.store.get_skills_needing_review(days=days)

    def suggest_mastery_upgrade(self, skill_name: str, recent_interactions: int) -> Optional[str]:
        """Suggest a mastery upgrade if the user has been active with a skill."""
        if recent_interactions < 3:
            return None
        if not self.store or not self.store._conn:
            return None
        row = self.store._conn.execute(
            "SELECT mastery_level FROM heimdall_skill_mastery WHERE skill_name = ?",
            (skill_name,),
        ).fetchone()
        if not row:
            return None
        current = row["mastery_level"]
        try:
            idx = MASTERY_LEVELS.index(current)
        except ValueError:
            return None
        if idx < len(MASTERY_LEVELS) - 1:
            next_level = MASTERY_LEVELS[idx + 1]
            return f"你最近{recent_interactions}次讨论了{skill_name}，要不要把掌握度从'{current}'提升到'{next_level}'？"
        return None

    # ------------------------------------------------------------------
    # System prompt formatting
    # ------------------------------------------------------------------

    def format_for_system_prompt(self, max_chars: int = 3000) -> str:
        """Format knowledge context for the system prompt."""
        content = self.snapshot
        if len(content) <= max_chars:
            return content
        return content[:max_chars]
