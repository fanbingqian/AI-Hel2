"""HEIMDALL Empty States — cold-start view messages.

From the V3.0 specification, each view has a friendly empty state
that guides the user toward creating content.
"""


class EmptyStates:
    """Empty state messages for each HEIMDALL view."""

    MESSAGES = {
        "recent": "还没有记录——开始聊天，你的记忆会出现在这里 📝",
        "history": "聊得越多，你的时间线越丰富 🌱",
        "groups": "当你拥有足够多的记忆后（至少5条相关记忆），它们会自动归类 🗂️",
        "learning": "随着你学习新东西，这里会记录你的成长 📈",
        "summary": "月底会有你的第一份个人成长报告 🎉",
    }

    ACTIONS = {
        "recent": "[ 开始对话 ]",
        "history": "[ 开始聊天 ]",
        "groups": "需要至少5条相关记忆才会自动激活分组",
        "learning": "AI 会在你学习新技能时自动跟踪",
        "summary": "每月自动生成，无需手动操作",
    }

    @classmethod
    def get(cls, view: str) -> str:
        """Get the empty state message for a view."""
        return cls.MESSAGES.get(view, "还没有内容 📝")

    @classmethod
    def get_action(cls, view: str) -> str:
        """Get the suggested action for a view."""
        return cls.ACTIONS.get(view, "")

    @classmethod
    def get_all(cls) -> dict:
        """Get all empty states."""
        return {
            view: {"message": cls.get(view), "action": cls.get_action(view)}
            for view in cls.MESSAGES
        }

    @classmethod
    def get_combined(cls, view: str) -> str:
        """Get a combined message+action block for display."""
        msg = cls.get(view)
        action = cls.get_action(view)
        if action:
            return f"{msg}\n{action}"
        return msg
