"""Fix community detection: redo backfill with proper session IDs."""
from hermes_constants import get_heimdall_dir
import sqlite3, time, random
from collections import defaultdict

db = get_heimdall_dir() / 'heimdall.db'
conn = sqlite3.connect(str(db))
conn.row_factory = sqlite3.Row

# 1. Delete old backfill edges
conn.execute("DELETE FROM heimdall_memory_edges WHERE session_id = 'backfill'")
conn.execute("DELETE FROM heimdall_social_graph WHERE relationship_type = 'mentioned_with' AND evidence_count = 1")
print("Deleted old backfill edges")

# 2. Load entities grouped by source_session_id
rows = conn.execute(
    "SELECT entity_id, source_session_id FROM heimdall_entities WHERE status = 'active'"
).fetchall()
sessions = defaultdict(list)
for r in rows:
    sid = r['source_session_id'] or 'unknown'
    sessions[sid].append(r['entity_id'])

print("{} entities in {} sessions".format(sum(len(v) for v in sessions.values()), len(sessions)))

# 3. Create edges within each session
now = time.time()
random.seed(42)
MAX_PER_ENTITY = 8
mem_added = 0
soc_added = 0

for sid, entity_ids in sessions.items():
    if len(entity_ids) < 2:
        continue
    for i, src_id in enumerate(entity_ids):
        candidates = [eid for j, eid in enumerate(entity_ids) if j != i]
        neighbors = random.sample(candidates, min(MAX_PER_ENTITY, len(candidates)))
        for tgt_id in neighbors:
            a, b = (src_id, tgt_id) if src_id < tgt_id else (tgt_id, src_id)
            # Check for existing social edge
            existing = conn.execute(
                "SELECT id FROM heimdall_social_graph WHERE source_entity_id = ? AND target_entity_id = ?",
                (a, b)
            ).fetchone()
            if existing:
                continue
            # Memory edge with ACTUAL source session (not "backfill")
            conn.execute(
                "INSERT INTO heimdall_memory_edges (entity_id, role, emotion, timestamp, session_id) "
                "VALUES (?, 'context', 0.0, ?, ?)", (src_id, now, sid))
            conn.execute(
                "INSERT INTO heimdall_memory_edges (entity_id, role, emotion, timestamp, session_id) "
                "VALUES (?, 'context', 0.0, ?, ?)", (tgt_id, now, sid))
            # Social edge
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
print("Added {} memory edges, {} social edges".format(mem_added, soc_added))
print("Final: {} memory edges, {} social edges".format(final_mem, final_soc))

# 4. Reset community assignments so re-detection works
conn.execute("UPDATE heimdall_entities SET community_id = NULL, community_confidence = 0.5, is_bridge = 0, bridge_score = 0 WHERE status = 'active'")
conn.commit()
print("Reset community assignments")
conn.close()
print("Done!")
