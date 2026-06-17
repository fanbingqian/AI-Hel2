"""HEIMDALL Daily Report Engine — Knowledge Ring V1.0.

Generates daily, weekly, and monthly growth reports from the kr_event_log table.
Provides domain statistics, streak calculation, and new domain detection.
"""

from __future__ import annotations

import logging
from datetime import date as dt_date, timedelta
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)


class DailyReportEngine:
    """Generates daily/weekly/monthly knowledge growth reports.

    Reads from kr_event_log, kr_entities, kr_relations, and kr_domain_first_seen
    tables to produce structured report data consumed by the web API and frontend.
    """

    def __init__(self, store: EntityStore):
        self.store = store

    # ------------------------------------------------------------------
    # Daily Report
    # ------------------------------------------------------------------

    def generate_daily_report(self, target_date: Optional[str] = None) -> dict:
        """Generate a daily growth report.

        Returns:
            {"date": str, "new_entities": int, "new_relations": int,
             "domain_growth": [...], "new_domains": [...],
             "social_updates": [...], "project_milestones": [...],
             "totals": {...}, "streak": int}
        """
        d = target_date or dt_date.today().isoformat()
        events = self.store.get_daily_events(d)

        new_entities = sum(1 for e in events if e["event_type"] == "entity_created")
        new_relations = sum(1 for e in events if e["event_type"] == "relation_added")
        corrections = sum(1 for e in events if e["event_type"] in ("field_edited", "user_correction"))

        # Domain growth from entities created today
        domain_growth = self._aggregate_domain_growth(events)

        # New domains detected today
        new_domains = self._detect_new_domains(d)

        # Social updates (knows relations added today)
        social_updates = self._extract_social_updates(events)

        # Project milestones
        project_milestones = self._extract_project_milestones(events)

        totals = {
            "entities": self.store.get_entity_count_v2(),
            "relations": self.store.get_relation_count(),
            "domains": len(self.store.get_domains()),
        }
        streak = self.store.calculate_streak()

        return {
            "date": d,
            "new_entities": new_entities,
            "new_relations": new_relations,
            "corrections": corrections,
            "domain_growth": domain_growth,
            "new_domains": new_domains,
            "social_updates": social_updates,
            "project_milestones": project_milestones,
            "totals": totals,
            "streak": streak,
        }

    # ------------------------------------------------------------------
    # Weekly Report
    # ------------------------------------------------------------------

    def generate_weekly_report(self, week_start: Optional[str] = None) -> dict:
        """Generate a weekly growth summary."""
        if week_start:
            start = dt_date.fromisoformat(week_start)
        else:
            today = dt_date.today()
            start = today - timedelta(days=today.weekday())
        end = start + timedelta(days=6)

        events = self.store.get_event_date_range(
            start.isoformat(), end.isoformat()
        )

        daily_breakdown = {}
        for i in range(7):
            d = (start + timedelta(days=i)).isoformat()
            day_events = [e for e in events if e["date"] == d]
            daily_breakdown[d] = {
                "entities": sum(1 for e in day_events if e["event_type"] == "entity_created"),
                "relations": sum(1 for e in day_events if e["event_type"] == "relation_added"),
            }

        return {
            "week_start": start.isoformat(),
            "week_end": end.isoformat(),
            "total_entities": sum(v["entities"] for v in daily_breakdown.values()),
            "total_relations": sum(v["relations"] for v in daily_breakdown.values()),
            "daily_breakdown": daily_breakdown,
            "domains": self._aggregate_domain_growth(events),
        }

    # ------------------------------------------------------------------
    # Monthly Report
    # ------------------------------------------------------------------

    def generate_monthly_report(self, month: Optional[str] = None) -> dict:
        """Generate a monthly growth summary.

        Args:
            month: ISO format 'YYYY-MM', defaults to current month.
        """
        if month:
            year, mon = month.split("-")
            year_int, mon_int = int(year), int(mon)
        else:
            today = dt_date.today()
            year_int, mon_int = today.year, today.month

        start = dt_date(year_int, mon_int, 1)
        if mon_int == 12:
            end = dt_date(year_int + 1, 1, 1) - timedelta(days=1)
        else:
            end = dt_date(year_int, mon_int + 1, 1) - timedelta(days=1)

        events = self.store.get_event_date_range(start.isoformat(), end.isoformat())

        domain_stats = self.store.get_domain_stats()

        return {
            "month": f"{year_int}-{mon_int:02d}",
            "start_date": start.isoformat(),
            "end_date": end.isoformat(),
            "total_new_entities": sum(1 for e in events if e["event_type"] == "entity_created"),
            "total_new_relations": sum(1 for e in events if e["event_type"] == "relation_added"),
            "total_corrections": sum(1 for e in events if e["event_type"] in ("field_edited", "user_correction")),
            "domain_distribution": domain_stats,
            "domain_growth": self._aggregate_domain_growth(events),
            "new_domains": self._detect_new_domains_in_range(start.isoformat(), end.isoformat()),
        }

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _aggregate_domain_growth(self, events: list[dict]) -> list[dict]:
        """Aggregate entity creation counts by domain from event log."""
        domain_counts: dict[str, int] = {}
        for e in events:
            if e["event_type"] == "entity_created" and e.get("entity_id"):
                entity = self.store.get_entity_v2(e["entity_id"])
                if entity and entity.get("domains"):
                    import json
                    try:
                        domains = json.loads(entity["domains"]) if isinstance(entity["domains"], str) else entity["domains"]
                    except (json.JSONDecodeError, TypeError):
                        domains = []
                    for dom in domains:
                        domain_counts[dom] = domain_counts.get(dom, 0) + 1
        return [
            {"domain": dom, "count": cnt}
            for dom, cnt in sorted(domain_counts.items(), key=lambda x: x[1], reverse=True)
        ]

    def _detect_new_domains(self, target_date: str) -> list[dict]:
        """Find domains first seen on the given date."""
        domains = self.store.get_domains()
        return [
            {"domain": d["domain_name"], "status": d.get("status", "auto_created")}
            for d in domains
            if d.get("first_seen") == target_date
        ]

    def _detect_new_domains_in_range(self, start_date: str, end_date: str) -> list[dict]:
        """Find domains first seen within a date range."""
        domains = self.store.get_domains()
        return [
            {"domain": d["domain_name"], "first_seen": d.get("first_seen", ""),
             "status": d.get("status", "auto_created")}
            for d in domains
            if start_date <= (d.get("first_seen") or "") <= end_date
        ]

    def _extract_social_updates(self, events: list[dict]) -> list[dict]:
        """Extract social (knows) relation additions from events."""
        updates = []
        for e in events:
            if e["event_type"] == "relation_added" and e.get("description", "").startswith("knows:"):
                updates.append({
                    "description": e.get("description", ""),
                    "entity_id": e.get("entity_id"),
                    "timestamp": str(e.get("timestamp", "")),
                })
        return updates[:10]

    def _extract_project_milestones(self, events: list[dict]) -> list[dict]:
        """Extract project-related events from event log."""
        milestones = []
        for e in events:
            if e.get("entity_id"):
                entity = self.store.get_entity_v2(e["entity_id"])
                if entity and entity.get("type_detail") == "project":
                    milestones.append({
                        "entity_id": e["entity_id"],
                        "name": entity.get("name", ""),
                        "event": e.get("event_type", ""),
                        "description": e.get("description", ""),
                        "timestamp": str(e.get("timestamp", "")),
                    })
        return milestones[:20]


# Module-level convenience functions
_engine: Optional[DailyReportEngine] = None


def set_daily_engine(store: EntityStore) -> DailyReportEngine:
    """Initialize or replace the global daily report engine."""
    global _engine
    _engine = DailyReportEngine(store)
    return _engine


def get_daily_report(date: Optional[str] = None) -> dict:
    """Convenience: generate daily report from the global engine."""
    if not _engine:
        return {"error": "DailyReportEngine not initialized"}
    return _engine.generate_daily_report(date)


def get_weekly_report(week_start: Optional[str] = None) -> dict:
    """Convenience: generate weekly report from the global engine."""
    if not _engine:
        return {"error": "DailyReportEngine not initialized"}
    return _engine.generate_weekly_report(week_start)


def get_monthly_report(month: Optional[str] = None) -> dict:
    """Convenience: generate monthly report from the global engine."""
    if not _engine:
        return {"error": "DailyReportEngine not initialized"}
    return _engine.generate_monthly_report(month)
