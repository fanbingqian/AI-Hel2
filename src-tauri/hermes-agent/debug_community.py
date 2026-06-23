import logging, sys
logging.basicConfig(level=logging.DEBUG, stream=sys.stderr)

from heimdall.web.api import get_heimdall
from heimdall.core.community import CommunityDetector

hm = get_heimdall()
gs = hm.provider.store

# Check session grouping
rows = gs._conn.execute(
    "SELECT source_session_id, COUNT(*) as cnt FROM heimdall_entities "
    "WHERE status='active' GROUP BY source_session_id ORDER BY cnt DESC"
).fetchall()
print("=== Session distribution ===")
for r in rows:
    print("  {}: {} entities".format(r['source_session_id'], r['cnt']))

# Check memory edge distribution
mem_rows = gs._conn.execute(
    "SELECT session_id, COUNT(*) as cnt FROM heimdall_memory_edges GROUP BY session_id ORDER BY cnt DESC"
).fetchall()
print("=== Memory edge sessions ===")
for r in mem_rows:
    print("  {}: {} edges".format(r['session_id'], r['cnt']))

soc = gs._conn.execute("SELECT COUNT(*) FROM heimdall_social_graph").fetchone()[0]
print("Social edges total: {}".format(soc))
