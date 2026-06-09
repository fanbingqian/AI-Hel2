import sqlite3, os
db = os.path.expanduser('~/.hermes/heimdall/heimdall.db')
conn = sqlite3.connect(db)
c = conn.cursor()

# Fast column query
for tbl in ['kr_relations', 'kr_entities', 'kr_causal_chains', 'kr_inferences', 'kr_namespaces']:
    c.execute(f'PRAGMA table_info({tbl})')
    cols = c.fetchall()
    print(f'=== {tbl} ===')
    for col in cols:
        print(f'  {col[1]:25s} {col[2]}')
    print()

# Also check if the column might be named differently
c.execute('SELECT * FROM kr_relations LIMIT 3')
desc = c.description
print('kr_relations actual columns:', [d[0] for d in desc])
for row in c.fetchall():
    print('  sample:', row)

conn.close()
