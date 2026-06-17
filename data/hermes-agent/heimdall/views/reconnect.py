"""HEIMDALL Reconnect Engine — social card generation.

Detects dormant social connections and generates shareable reconnect
cards. Reconnect suggestions are derived from the social graph's
3D model (intensity/valence/volatility).

Reconnect cards are designed to be:
  - De-identified (no real names exposed)
  - Shareable on social platforms
  - Warm and human, not robotic
"""

from __future__ import annotations

import logging
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

RECONNECT_CARD_TEMPLATE = """我可能忘记了一些事情，但我的AI提醒我，我们已经{days}天没联系了。
最近还好吗？❤️
#AI守护 #重要的关系"""


class ReconnectEngine:
    """Generates reconnect suggestions and shareable social cards."""

    def __init__(self, store: EntityStore):
        self.store = store

    def get_suggestions(self) -> list[dict]:
        """Get reconnect suggestions from the social graph.

        Returns list of suggested reconnections with:
          - entity info (de-identified)
          - days inactive
          - relationship health score
          - suggested message
          - shareable card text
        """
        edges = self.store.get_reconnect_suggestions()
        suggestions = []
        for edge in edges:
            import time
            days_inactive = (time.time() - edge.get("last_seen", 0)) / 86400
            suggestions.append({
                "entity_id": edge.get("target_entity_id"),
                "days_inactive": round(days_inactive),
                "intensity": edge.get("intensity", 0),
                "valence": edge.get("valence", 0),
                "health_score": edge.get("health_score", 0.5),
                "message": self._build_reconnect_message(
                    days_inactive=round(days_inactive),
                    intensity=edge.get("intensity", 0),
                ),
                "card": self.generate_card(
                    days_inactive=round(days_inactive),
                    intensity=edge.get("intensity", 0),
                ),
            })
        return suggestions

    def generate_card(self, days_inactive: int, intensity: float = 0.5) -> str:
        """Generate a shareable reconnect card.

        The card is de-identified — no real names, just the sentiment.
        """
        return RECONNECT_CARD_TEMPLATE.format(days=days_inactive)

    def _build_reconnect_message(self, days_inactive: int, intensity: float = 0.5) -> str:
        """Build a warm, human reconnect message."""
        if intensity > 0.7:
            template = (
                "💬 你好像有一阵没联系某位重要的联系人了——"
                "上次联系是{days}天前，以前你们经常聊天。"
            )
        elif intensity > 0.3:
            template = (
                "👋 已经{days}天没和某位联系人互动了，"
                "要不要问候一下？"
            )
        else:
            template = (
                "📝 你有一段时间没联系一位老朋友了（{days}天）。"
            )
        return template.format(days=days_inactive)

    def get_card_actions(self, entity_id: str) -> list[dict]:
        """Get available actions for a reconnect card."""
        return [
            {"action": "say_hi", "label": "说点什么"},
            {"action": "generate_card", "label": "生成卡片分享"},
            {"action": "ignore", "label": "忽略"},
            {"action": "snooze", "label": "30天后提醒"},
        ]
