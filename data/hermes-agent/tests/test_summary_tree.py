"""Tests for P2.1 summary tree engine."""
import pytest
from datetime import date
from heimdall.core.entity_store.summary_tree import (
    SummaryTreeEngine,
    MAX_SUMMARY_CHARS,
    L0_TO_L1_THRESHOLD,
    L1_TO_L2_THRESHOLD,
    L2_TO_L3_THRESHOLD,
    LEVEL_NAMES,
    SUMMARY_TREE_SCHEMA,
)


class TestSummaryTreeEngine:
    def test_bootstrap_daily_creates_entry(self):
        """Bootstrap creates a pending L0 entry."""
        class FakeStore:
            _conn = None
        engine = SummaryTreeEngine(FakeStore())
        # No DB, should return None gracefully
        result = engine.bootstrap_daily(target_date=date.today())
        assert result is None

    def test_cascade_without_db(self):
        """Cascade with no DB returns empty dict."""
        class FakeStore:
            _conn = None
        engine = SummaryTreeEngine(FakeStore())
        results = engine.cascade()
        assert results == {}

    def test_get_summaries_without_db(self):
        """get_summaries with no DB returns empty list."""
        class FakeStore:
            _conn = None
        engine = SummaryTreeEngine(FakeStore())
        assert engine.get_summaries() == []
        assert engine.get_latest_summary() is None

    def test_generate_without_llm(self):
        """generate_summary without LLM callable returns False."""
        class FakeStore:
            _conn = None
        engine = SummaryTreeEngine(FakeStore())
        assert engine.generate_summary(1) is False

    def test_generate_without_db(self):
        """generate_summary with LLM but no DB returns False."""
        class FakeStore:
            _conn = None
        calls = []
        engine = SummaryTreeEngine(FakeStore(), llm_call=lambda sp, up: calls.append((sp, up)) or "ok")
        assert engine.generate_summary(1) is False
        assert calls == []

    def test_initialize_schema_no_db(self):
        """initialize_schema with no DB does not crash."""
        class FakeStore:
            _conn = None
        engine = SummaryTreeEngine(FakeStore())
        engine.initialize_schema()  # should not raise

    def test_constants(self):
        assert L0_TO_L1_THRESHOLD == 7
        assert L1_TO_L2_THRESHOLD == 4
        assert L2_TO_L3_THRESHOLD == 12
        assert LEVEL_NAMES[0] == "daily"
        assert LEVEL_NAMES[3] == "yearly"
        assert MAX_SUMMARY_CHARS == 5000

    def test_schema_sql_is_valid(self):
        """Schema DDL is a non-empty string with expected keywords."""
        assert "CREATE TABLE" in SUMMARY_TREE_SCHEMA
        assert "summary_tree" in SUMMARY_TREE_SCHEMA
        assert "CREATE INDEX" in SUMMARY_TREE_SCHEMA
