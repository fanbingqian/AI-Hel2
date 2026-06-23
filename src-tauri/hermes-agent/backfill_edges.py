"""Backfill co-occurrence edges for all existing entities.
Groups entities by source_session_id and creates edges within each group.
Each entity connects to up to 10 others in its session.
"""
from hermes_constants import get_heimdall_dir
import sqlite3, time, random
from collections import defaultdict

db = get_heimdall_dir() / 'heimdall.db'
conn = sqlite3.connect(str(db))
conn.row_factory = sqlite3.Row

rows = conn.execute(
    "SELECT entity_id, display_name, source_session_id FROM heimdall_entities "
    "WHERE status = 'active'"
).fetchall()

# Group by session
sessions = defaultdict(list)
for r in rows:
    e = dict(r)
    sid = e.get('source_session_id', 'unknown')
    sessions[sid].append(e['entity_id'])

print(f"{len(rows)} entities in {len(sessions)} sessions")
existing_soc = conn.execute("SELECT COUNT(*) FROM heimdall_social_graph").fetchone()[0]
print(f"Existing social edges: {existing_soc}")

now = time.time()
random.seed(42)
MAX_PER_ENTITY = 8
mem_added = 0
soc_added = 0

for sid, entity_ids in sessions.items():
    if len(entity_ids) < 2:
        continue
    print(f"  Session {sid}: {len(entity_ids)} entities")
    for i, src_id in enumerate(entity_ids):
        candidates = [eid for j, eid in enumerate(entity_ids) if j != i]
        neighbors = random.sample(candidates, min(MAX_PER_ENTITY, len(candidates)))
        for tgt_id in neighbors:
            a, b = (src_id, tgt_id) if src_id < tgt_id else (tgt_id, src_id)
            existing = conn.execute(
                "SELECT id FROM heimdall_social_graph "
                "WHERE source_entity_id = ? AND target_entity_id = ?",
                (a, b)
            ).fetchone()
            if existing:
                continue
            conn.execute(
                "INSERT INTO heimdall_memory_edges (entity_id, role, emotion, timestamp, session_id) "
                "VALUES (?, 'context', 0.0, ?, 'backfill')", (src_id, now))
            conn.execute(
                "INSERT INTO heimdall_memory_edges (entity_id, role, emotion, timestamp, session_id) "
                "VALUES (?, 'context', 0.0, ?, 'backfill')", (tgt_id, now))
            conn.execute(
                "INSERT INTO heimdall_social_graph "
                "(source_entity_id, target_entity_id, relationship_type, intensity, valence, volatility, health_score, evidence_count, first_seen, last_seen) "
                "VALUES (?, ?, 'mentioned_with', 0.3, 0.0, 0.0, 0.5, 1, ?, ?)",
                (a, b, now, now))
            mem_added += 2
            soc_added += 1

conn.commit()

final_mem = conn.execute("SELECT COUNT(*) FROM heimdall_memory_edges").fetchone()[0]
final_soc = conn.execute("SELECT COUNT(*) FROM heimdall_social_graph").fetchone()[0]
print(f"Added {mem_added} memory edges, {soc_added} social edges")
print(f"Final: {final_mem} memory edges, {final_soc} social edges")
conn.close()
print("Done!")
