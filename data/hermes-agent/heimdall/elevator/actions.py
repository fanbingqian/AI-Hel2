"""HEIMDALL Action Suggester — L1/L2/L3 graded action suggestions.

Memory drives action — when the system detects patterns in entity
relationships, social graph signals, or knowledge mastery, it suggests
actions at the appropriate capability level.

Action levels correspond to the cognitive elevator:
  L1: Simple reminders, knowledge review prompts
  L2: Social reconnect suggestions, learning recommendations
  L3: Deep analysis, pivot moment narratives (cloud-gated)
"""

from __future__ import annotations

import logging
from typing import Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)


class ActionSuggester:
    """Generates graded action suggestions from memory and knowledge patterns."""

    def __init__(self, store: EntityStore):
        self.store = store

    # ------------------------------------------------------------------
    # L1 Actions (on-device, low-stakes)
    # ------------------------------------------------------------------

    def suggest_knowledge_review(self) -> list[dict]:
        """Suggest knowledge entries that need review (inactive >30 days)."""
        stale = self.store.get_stale_knowledge(days=30)
        return [
            {
                "level": "L1",
                "type": "knowledge_review",
                "title": entry.get("title", ""),
                "domain": entry.get("domain", ""),
                "message": f"📚 你可能需要复习'{entry.get('title', '')}'——已经超过30天没看过了。",
            }
            for entry in stale[:3]
        ]

    def suggest_skill_review(self) -> list[dict]:
        """Suggest skills that need practice (inactive >30 days)."""
        skills = self.store.get_skills_needing_review(days=30)
        return [
            {
                "level": "L1",
                "type": "skill_review",
                "skill": s.get("skill_name", ""),
                "domain": s.get("parent_domain", ""),
                "message": f"⏰ '{s.get('skill_name', '')}'有段时间没练习了，是否需要复习？",
            }
            for s in skills[:3]
        ]

    # ------------------------------------------------------------------
    # L2 Actions (buffer zone, moderate stakes)
    # ------------------------------------------------------------------

    def suggest_reconnect(self) -> list[dict]:
        """Suggest reconnection with dormant contacts.

        Uses social graph signals: inactive >90 days, ≥10 past interactions.
        """
        if not self.store._conn:
            return []
        edges = self.store.get_reconnect_suggestions()
        suggestions = []
        for edge in edges:
            import time
            days = (time.time() - edge.get("last_seen", 0)) / 86400
            suggestions.append({
                "level": "L2",
                "type": "reconnect",
                "entity_id": edge.get("target_entity_id", ""),
                "name": edge.get("target_name", ""),
                "days_inactive": round(days),
                "intensity": edge.get("intensity", 0),
                "message": (
                    f"💬 你好像有一阵没联系{edge.get('target_name', '某位联系人')}了——"
                    f"上次联系是{round(days)}天前，以前你们经常聊天。"
                ),
            })
        return suggestions[:3]

    def suggest_mastery_upgrade(self, skill_name: str, recent_interactions: int) -> Optional[dict]:
        """Suggest upgrading a skill's mastery level."""
        if recent_interactions < 3:
            return None
        return {
            "level": "L2",
            "type": "mastery_upgrade",
            "skill": skill_name,
            "interactions": recent_interactions,
            "message": f"你最近{recent_interactions}次讨论了'{skill_name}'，掌握度是否需要提升？",
        }

    # ------------------------------------------------------------------
    # L3 Actions (deep water, cloud-gated)
    # ------------------------------------------------------------------

    def suggest_deep_analysis(self, topic: str, context: str = "") -> Optional[dict]:
        """Suggest a deep analysis action (L3 — requires cloud escalation)."""
        return {
            "level": "L3",
            "type": "deep_analysis",
            "topic": topic,
            "context": context[:200],
            "message": f"🔍 '{topic}'可能需要更深入的分析。需要我连接云端进行深度分析吗？",
        }

    # ------------------------------------------------------------------
    # Aggregation
    # ------------------------------------------------------------------

    def get_all_suggestions(self, max_per_level: int = 3) -> dict:
        """Get all suggestions grouped by level."""
        suggestions = {"L1": [], "L2": [], "L3": []}

        l1 = self.suggest_knowledge_review() + self.suggest_skill_review()
        suggestions["L1"] = l1[:max_per_level]

        l2 = self.suggest_reconnect()
        suggestions["L2"] = l2[:max_per_level]

        return suggestions
