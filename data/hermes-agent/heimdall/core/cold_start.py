"""HEIMDALL Cold Start — first-conversation profiles and legacy data migration.

Cold start flow (from V3.0 spec):
  1. Value declaration (3s): "This is an AI that remembers you"
  2. Quick authorization (5s): notification/background permissions
  3. Persona guidance (2min): simplified 3-step — who am I, what I care about, what to call me
  4. First conversation: AI actively asks engaging questions, first memory within 30s

Migration from Hermes Agent:
  - Read old MEMORY.md / USER.md
  - Extract entities using EntityExtractor
  - Populate HEIMDALL entity store
  - Write PERSONA.md / KNOWLEDGE.md
  - Generate privacy salt
  - Create migration marker file
"""

from __future__ import annotations

import logging
import time
from pathlib import Path
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore
from heimdall.core.extraction import EntityExtractor
from heimdall.core.persona import PersonaManager
from heimdall.core.knowledge import KnowledgeManager
from heimdall.core.privacy import load_or_create_salt

logger = logging.getLogger(__name__)

COLD_START_QUESTIONS = [
    "嗨，我已经准备好了。你刚才说你是{role}，我很好奇，最近最让你兴奋的一个项目是什么？",
    "说到{interest}，你是怎么开始对这个感兴趣的？有什么特别的故事吗？",
    "你提到的{entity}听起来很重要——能多跟我说说吗？",
]

MIGRATION_MARKER = ".migrated_from_hermes"

EMPTY_STATE_MESSAGES = {
    "recent": "还没有记录——开始聊天，你的记忆会出现在这里 📝",
    "history": "聊得越多，你的时间线越丰富 🌱",
    "groups": "当你拥有足够多的记忆后（至少5条相关记忆），它们会自动归类 🗂️",
    "learning": "随着你学习新东西，这里会记录你的成长 📈",
    "summary": "月底会有你的第一份个人成长报告 🎉",
}


class ColdStartExtractor:
    """Handles first-run persona creation and legacy data migration."""

    def __init__(
        self,
        store: EntityStore,
        persona: PersonaManager,
        knowledge: KnowledgeManager,
        extractor: EntityExtractor,
        heimdall_dir: Path,
    ):
        self.store = store
        self.persona = persona
        self.knowledge = knowledge
        self.extractor = extractor
        self.heimdall_dir = heimdall_dir

    # ------------------------------------------------------------------
    # Cold start detection
    # ------------------------------------------------------------------

    def is_first_run(self) -> bool:
        """Check if this is the first HEIMDALL run."""
        return self.store.get_entity_count() == 0

    def is_migrated_from_hermes(self) -> bool:
        """Check if migration from Hermes Agent has already been done."""
        return (self.heimdall_dir / MIGRATION_MARKER).exists()

    # ------------------------------------------------------------------
    # Cold start profile
    # ------------------------------------------------------------------

    def generate_initial_persona(
        self,
        identity: str = "",
        values: list[str] = None,
        role: str = "",
    ) -> str:
        """Generate the initial PERSONA.md from cold-start responses.

        Called after the user answers the 3-step cold-start questions.
        """
        values = values or []
        persona_content = self.persona.load()
        persona_content = persona_content.replace(
            "- 身份: [一句话描述你是谁]",
            f"- 身份: {identity or '探索者'}"
        )
        persona_content = persona_content.replace(
            "- 价值观:\n  1. [价值观1]\n  2. [价值观2]\n  3. [价值观3]",
            "- 价值观:\n" + "\n".join(f"  {i+1}. {v}" for i, v in enumerate(values[:5]))
            if values else "- 价值观: [待探索]"
        )
        self.persona.persona_path.write_text(persona_content, encoding="utf-8")
        self.persona.load()
        return persona_content

    def get_first_message_entities(self, user_response: str, session_id: str = "") -> dict:
        """Extract entities from the user's first response.

        This should run within 30 seconds of the first conversation.
        """
        result = self.extractor.extract_from_text(user_response, session_id)
        logger.info("Cold start: extracted %d entities from first response", result.get("count", 0))
        return result

    def generate_first_memory_summary(self, entities: list[dict]) -> str:
        """Generate the 'I remembered' summary for the first conversation."""
        if not entities:
            return "我已经记住了你说的。"
        names = [e.get("name", "") for e in entities[:3]]
        if len(names) == 1:
            return f"我已经记住了你说的，特别是关于{names[0]}。"
        elif len(names) == 2:
            return f"我已经记住了你说的，特别是关于{names[0]}和{names[1]}。"
        else:
            return f"我已经记住了你说的，特别是关于{names[0]}、{names[1]}和{names[2]}。"

    # ------------------------------------------------------------------
    # Migration from Hermes Agent
    # ------------------------------------------------------------------

    def migrate_from_hermes(self, hermes_home: Path) -> dict:
        """Migrate data from Hermes Agent's MEMORY.md and USER.md.

        Args:
            hermes_home: Path to the old ~/.hermes directory.

        Returns:
            {"entities_migrated": N, "knowledge_migrated": N, "salt_generated": bool}
        """
        memory_dir = hermes_home / "memories"
        memory_path = memory_dir / "MEMORY.md"
        user_path = memory_dir / "USER.md"

        result = {"entities_migrated": 0, "knowledge_migrated": 0, "salt_generated": False}

        # Generate salt
        salt_path = self.heimdall_dir / ".salt"
        salt = load_or_create_salt(salt_path)
        if salt:
            result["salt_generated"] = True

        # Extract from old memory files
        all_text = ""
        for path in (memory_path, user_path):
            if path.exists():
                all_text += path.read_text(encoding="utf-8") + "\n"

        if all_text.strip():
            extracted = self.extractor.extract_from_text(all_text, "migration")
            result["entities_migrated"] = extracted.get("count", 0)

            # Old MEMORY.md entries → knowledge entries
            if memory_path.exists():
                entries = self._parse_old_memory_entries(memory_path)
                for entry in entries:
                    self.knowledge.add_entry(
                        domain="导入",
                        title=f"从Hermes导入: {entry[:50]}",
                        content=entry,
                        confidence=0.5,
                        source_type="migration",
                    )
                    result["knowledge_migrated"] += 1

            # Write migration marker
            (self.heimdall_dir / MIGRATION_MARKER).write_text(
                str(time.time()), encoding="utf-8"
            )

        return result

    def _parse_old_memory_entries(self, path: Path) -> list[str]:
        """Parse old Hermes MEMORY.md entries (separated by § delimiter)."""
        if not path.exists():
            return []
        content = path.read_text(encoding="utf-8")
        entries = content.split("\n§\n")
        return [e.strip() for e in entries if e.strip()]

    # ------------------------------------------------------------------
    # Empty states
    # ------------------------------------------------------------------

    @staticmethod
    def get_empty_state(view: str) -> str:
        """Get the empty state message for a view."""
        return EMPTY_STATE_MESSAGES.get(view, "还没有内容 📝")

    @staticmethod
    def get_all_empty_states() -> dict:
        """Get all empty state messages."""
        return dict(EMPTY_STATE_MESSAGES)
