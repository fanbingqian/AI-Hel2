import sqlite3, os
db = os.path.expanduser('~/.hermes/heimdall/heimdall.db')
conn = sqlite3.connect(db)
c = conn.cursor()

for tbl in ['kr_relations', 'kr_entities', 'heimdall_knowledge_edges', 'heimdall_knowledge_entries', 'kr_causal_chains']:
    c.execute('SELECT sql FROM sqlite_master WHERE name=?', (tbl,))
    row = c.fetchone()
    if row:
        sql = row[0].replace('\n', ' | ')
        print(f'{tbl}: {sql[:400]}')
    else:
        print(f'{tbl}: NOT FOUND')
    print()
conn.close()
