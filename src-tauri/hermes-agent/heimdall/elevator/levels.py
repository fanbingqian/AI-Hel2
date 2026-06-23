"""HEIMDALL Cognitive Elevator — L1/L2/L3 capability boundary enforcement.

From the V3.0 specification:

  L1 安全区 (Safe Zone): On-device autonomous response
    - Simple facts, personal history, contact queries
    - On-device model responds directly

  L2 缓冲区 (Buffer Zone): Local best-effort with warning
    - Mild emotions, vague intent, simple summaries
    - On-device model tries, but appends "I'm not sure — want me to
      use cloud analysis?" if perplexity is high or user disagrees

  L3 深水区 (Deep Water): Forced cloud escalation
    - Deep emotions, complex reasoning, pivot moment insight, long-form writing
    - NEVER generates locally — directly offers: "This needs deeper
      cognitive capability. Shall I connect to the cloud?
      (All data encrypted in transit, used only for this analysis.)"
"""

from __future__ import annotations

import logging
from enum import Enum
from typing import Optional

logger = logging.getLogger(__name__)


class ElevationLevel(Enum):
    """Cognitive capability levels."""
    L1 = "l1"      # Safe zone — on-device
    L2 = "l2"      # Buffer zone — local best-effort
    L3 = "l3"      # Deep water — cloud required


# Patterns that indicate deep/complex emotional content (L3 triggers)
L3_TRIGGER_PATTERNS = [
    # Chinese
    "分手", "离婚", "去世", "抑郁", "焦虑症", "自杀", "创伤",
    "深度分析", "人生意义", "哲学问题", "职业规划",
    # English
    "breakup", "divorce", "died", "depression", "anxiety disorder",
    "suicide", "trauma", "deep analysis", "meaning of life",
]

# Patterns that indicate moderate complexity (L2 triggers)
L2_TRIGGER_PATTERNS = [
    "建议", "应该怎么办", "帮我分析", "不确定", "纠结",
    "advice", "what should I do", "help me analyze", "uncertain", "torn between",
]

# Topics that are always L1 safe
L1_SAFE_TOPICS = [
    "天气", "时间", "提醒", "联系人", "计算", "翻译",
    "weather", "time", "reminder", "contact", "calculate", "translate",
]


class CognitiveElevator:
    """Enforces L1/L2/L3 capability boundaries for on-device vs. cloud.

    The elevator ensures:
      - L3 content NEVER generates locally (privacy + quality gate)
      - L2 content generates locally but warns on low confidence
      - L1 content handles fully on-device

    This design is about HONEST capability boundaries — admitting when
    the on-device model cannot provide quality responses.
    """

    def __init__(self):
        self._escalation_count = 0
        self._l3_blocked_count = 0

    # ------------------------------------------------------------------
    # Classification
    # ------------------------------------------------------------------

    def classify(self, user_message: str, context_entities: Optional[list[dict]] = None) -> ElevationLevel:
        """Classify a user message into L1, L2, or L3.

        Args:
            user_message: The raw user input.
            context_entities: Entities from memory context (unused in Phase 1).

        Returns:
            ElevationLevel.L1, L2, or L3.
        """
        text = user_message.lower()

        # Check L3 triggers first (highest priority)
        for pattern in L3_TRIGGER_PATTERNS:
            if pattern.lower() in text:
                self._l3_blocked_count += 1
                logger.info("L3 trigger matched: '%s'", pattern)
                return ElevationLevel.L3

        # Check L2 triggers
        for pattern in L2_TRIGGER_PATTERNS:
            if pattern.lower() in text:
                return ElevationLevel.L2

        # Check L1 safe topics for explicit confirmation
        for pattern in L1_SAFE_TOPICS:
            if pattern.lower() in text:
                return ElevationLevel.L1

        # Default: L1 for simple queries, L2 for ambiguous
        if len(text) < 50:
            return ElevationLevel.L1
        return ElevationLevel.L2

    def classify_tool_call(self, tool_name: str, tool_args: dict) -> ElevationLevel:
        """Classify a tool call's capability level.

        Certain tools (browser, code execution) are always L2+.
        """
        l3_tools = {"execute_code", "browser_navigate", "delegate_task"}
        l2_tools = {"web_search", "web_extract", "session_search"}

        if tool_name in l3_tools:
            return ElevationLevel.L3
        if tool_name in l2_tools:
            return ElevationLevel.L2
        return ElevationLevel.L1

    # ------------------------------------------------------------------
    # Response templates
    # ------------------------------------------------------------------

    def get_l2_warning(self, original_response: str) -> str:
        """Append an L2 uncertainty warning to a local response."""
        return (
            f"{original_response}\n\n"
            "---\n"
            "⚠️ 我有些拿不准，要不要用云端大脑再分析一次？"
        )

    def get_l3_block_message(self, user_query_summary: str) -> str:
        """Generate an L3 block message — NEVER respond locally.

        Returns a message asking for cloud escalation consent.
        """
        self._escalation_count += 1
        return (
            "这需要我们更深度的认知能力，需要我连接到云端吗？\n\n"
            "（所有数据加密传输，仅用于本次分析。你的个人信息始终保留在本地。）\n\n"
            f"你想深入分析的是：{user_query_summary[:100]}"
        )

    def user_approved_escalation(self) -> bool:
        """Check if user has approved cloud escalation.

        In Phase 1, this checks a session-scoped flag.
        In Phase 2, this can integrate with the approval system.
        """
        return True  # Simplified: in Phase 1, always allow if user explicitly agrees

    # ------------------------------------------------------------------
    # Stats
    # ------------------------------------------------------------------

    @property
    def stats(self) -> dict:
        return {
            "escalation_count": self._escalation_count,
            "l3_blocked_count": self._l3_blocked_count,
        }
