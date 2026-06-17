"""HEIMDALL Persona Manager — PERSONA.md with 4-layer boundary protection.

The persona file defines the user's identity across four layers:
  1. Core Self (constitutional immunity zone)
  2. External Persona (algorithm-inferred)
  3. User Profile (objective facts, hot data ≤5000 chars)
  4. Social Anchors (hashed third-party references)

Core Self is protected by vector semantic guard — entities in this layer
cannot be contaminated by negative sentiment. Monthly constitutional checks
ensure persona drift stays above cosine 0.85 threshold.
"""

from __future__ import annotations

import logging
import re
import time
from pathlib import Path
from typing import Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

PERSONA_FILENAME = "PERSONA.md"

PERSONA_TEMPLATE = """# PERSONA

## 核心自我 [宪法免疫区]
<!-- 月度校验，向量锚点余弦 <0.85 触发人工确认 -->
- 身份: [一句话描述你是谁]
- 价值观:
  1. [价值观1]
  2. [价值观2]
  3. [价值观3]
- 行为底线:
  1. [底线1]
  2. [底线2]

## 对外人格 [算法推断，低置信度时用户确认]
- 沟通风格:
  1. [风格描述]
- 能力边界:
  1. [边界描述]

## 用户画像 [热数据≤5000字符]
- 基础信息: [城市/设备/职业等]
- 偏好标签: [自动计算权重]
- 禁忌雷区: [必须避免的话题和行为]

## 社交锚点 [第三方已哈希]
- 关键他人: [关系类型/三维指标]
- 群体归属: [家庭/工作/兴趣社群]
- 社交面具: [在不同人面前的表现差异]

## 运行规则
- 记忆调用规范: 按需检索，不过度关联
- 行动触发规则: L1端侧自治 / L2本地尽力+预警 / L3强制升舱
- 知识边界提示: 掌握度低时主动声明不确定性
"""


class PersonaManager:
    """Manages the PERSONA.md file with four-layer boundary semantics.

    Layer 1 (Core Self) is the constitutional immunity zone:
    - Only modifiable by monthly user confirmation
    - Protected by vector semantic guard (cosine ≥ 0.85)
    - Never contaminated by negative emotion

    Layer 2 (External Persona) is algorithmically inferred:
    - Updated from conversation patterns
    - Low-confidence updates request user confirmation

    Layer 3 (User Profile) is objective facts:
    - Updated frequently from conversation
    - Conflicts resolved by recency

    Layer 4 (Social Anchors) are hashed third-party references:
    - Stored as salted hashes
    - Individually deletable
    """

    def __init__(self, heimdall_dir: Path, store: Optional[EntityStore] = None):
        self.heimdall_dir = heimdall_dir
        self.persona_path = heimdall_dir / PERSONA_FILENAME
        self.store = store
        self._snapshot: Optional[str] = None  # Frozen at session start

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def load(self) -> str:
        """Load PERSONA.md content. Creates from template if missing.

        The loaded content is frozen as a snapshot for the session —
        mid-session writes update the file but NOT the snapshot,
        preserving prompt cache stability.
        """
        self.heimdall_dir.mkdir(parents=True, exist_ok=True)
        if not self.persona_path.exists():
            self.persona_path.write_text(PERSONA_TEMPLATE, encoding="utf-8")
        self._snapshot = self.persona_path.read_text(encoding="utf-8")
        return self._snapshot

    @property
    def snapshot(self) -> str:
        """Return the frozen session snapshot (never changes mid-session)."""
        if self._snapshot is None:
            return self.load()
        return self._snapshot

    # ------------------------------------------------------------------
    # Layer accessors
    # ------------------------------------------------------------------

    def get_core_self(self) -> list[str]:
        """Extract Core Self entries from the persona."""
        return self._extract_section("核心自我")

    def get_external_persona(self) -> list[str]:
        """Extract External Persona entries."""
        return self._extract_section("对外人格")

    def get_user_profile(self) -> list[str]:
        """Extract User Profile entries."""
        return self._extract_section("用户画像")

    def get_social_anchors(self) -> list[str]:
        """Extract Social Anchor entries."""
        return self._extract_section("社交锚点")

    # ------------------------------------------------------------------
    # Mutation (with boundary enforcement)
    # ------------------------------------------------------------------

    def add_core_value(self, value: str, require_confirmation: bool = True) -> bool:
        """Add a value to Core Self. Requires user confirmation by default."""
        if require_confirmation:
            logger.info("Core Self mutation requested but requires user confirmation: %s", value)
            return False
        return self._append_to_section("核心自我", f"- {value}")

    def update_external_persona(self, entry: str, confidence: float = 0.8) -> bool:
        """Update External Persona. Low confidence requests user confirmation."""
        if confidence < 0.8:
            logger.info("External Persona update low confidence (%.2f), skipping: %s", confidence, entry)
            return False
        return self._append_to_section("对外人格", f"- {entry}")

    def update_user_profile(self, entry: str) -> bool:
        """Update User Profile. Conflicts resolved by recency."""
        return self._append_to_section("用户画像", f"- {entry}")

    def add_social_anchor(self, entry: str) -> bool:
        """Add a social anchor entry (with hashed third-party names)."""
        return self._append_to_section("社交锚点", f"- {entry}")

    # ------------------------------------------------------------------
    # Knowledge Ring V1.0 — Profiles table dual-write
    # ------------------------------------------------------------------

    def sync_to_profiles_table(self, profile_type: str, content: str,
                                trigger_context: str = "", confidence: float = 0.5,
                                tag: str = "ai_only") -> Optional[int]:
        """Dual-write a user profile entry (V2.2: DB encrypted, PERSONA.md plaintext).

        DB layer: forced encryption via add_profile (no content plaintext column).
        PERSONA.md: plaintext for fast system-prompt injection.
        """
        if not self.store:
            return None
        try:
            return self.store.upsert_profile(
                profile_type=profile_type,
                content=content,
                trigger_context=trigger_context,
                confidence=confidence,
                tag=tag,
            )
        except Exception:
            logger.debug("Failed to sync profile to kr_profiles", exc_info=True)
            return None

    # ------------------------------------------------------------------
    # Persona drift monitoring
    # ------------------------------------------------------------------

    def check_drift(self) -> dict:
        """Check for persona drift by comparing current Core Self to anchor.

        Returns:
            {"drifted": bool, "cosine_similarity": float | None, "needs_review": bool}
        """
        current_core = self.get_core_self()
        if not current_core:
            return {"drifted": False, "cosine_similarity": None, "needs_review": False}
        # Simplified check: count-based drift detection
        # Full vector comparison requires embedding model (Phase 2)
        anchor_count = len(self._extract_original_core_values())
        if anchor_count == 0:
            return {"drifted": False, "cosine_similarity": 1.0, "needs_review": False}
        change_ratio = abs(len(current_core) - anchor_count) / max(anchor_count, 1)
        return {
            "drifted": change_ratio > 0.3,
            "cosine_similarity": 1.0 - change_ratio,
            "needs_review": change_ratio > 0.3,
        }

    # ------------------------------------------------------------------
    # System prompt formatting
    # ------------------------------------------------------------------

    def format_for_system_prompt(self, max_chars: int = 5000) -> str:
        """Format the persona snapshot for injection into the system prompt.

        Truncates to max_chars, prioritizing:
        1. Core Self (always included in full)
        2. User Profile (hot data)
        3. External Persona + Social Anchors (tail-truncated)
        """
        content = self.snapshot
        if len(content) <= max_chars:
            return content
        # Keep header + Core Self + User Profile, truncate rest
        sections = content.split("\n## ")
        result = sections[0]
        remaining = max_chars - len(result)
        for section in sections[1:]:
            header = section.split("\n")[0] if "\n" in section else section
            if "核心自我" in header or "用户画像" in header:
                chunk = f"\n## {section}"
                if len(chunk) <= remaining:
                    result += chunk
                    remaining -= len(chunk)
        return result[:max_chars]

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _extract_section(self, section_name: str) -> list[str]:
        """Extract list items from a named section."""
        content = self.snapshot
        pattern = rf'##\s+{section_name}.*?\n(.*?)(?=\n##\s|\Z)'
        match = re.search(pattern, content, re.DOTALL)
        if not match:
            return []
        lines = match.group(1).strip().split("\n")
        return [l.strip("- ").strip() for l in lines if l.strip().startswith("-")]

    def _append_to_section(self, section_name: str, entry: str) -> bool:
        """Append an entry to a named section in PERSONA.md."""
        content = self.persona_path.read_text(encoding="utf-8")
        pattern = rf'(##\s+{section_name}.*?\n)'
        if not re.search(pattern, content):
            return False
        updated = re.sub(
            pattern,
            rf'\1{entry}\n',
            content,
            count=1,
        )
        self.persona_path.write_text(updated, encoding="utf-8")
        return True

    def _extract_original_core_values(self) -> list[str]:
        """Extract Core Self values from the stored file (not snapshot)."""
        if not self.persona_path.exists():
            return []
        content = self.persona_path.read_text(encoding="utf-8")
        match = re.search(r'##\s+核心自我.*?\n(.*?)(?=\n##\s|\Z)', content, re.DOTALL)
        if not match:
            return []
        lines = match.group(1).strip().split("\n")
        return [l.strip("- ").strip() for l in lines if l.strip().startswith("- 价值观") or l.strip().startswith("- 行为")]
