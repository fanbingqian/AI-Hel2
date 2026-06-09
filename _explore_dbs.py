import sqlite3, os

dbs = {
    'knowledge_cache': r'C:\Users\58451\.hermes\knowledge_cache.db',
    'heimdall': r'C:\Users\58451\.hermes\heimdall\heimdall.db',
    'knowledge': r'C:\Users\58451\.hermes\knowledge\knowledge.db',
}

for name, path in dbs.items():
    size = os.path.getsize(path)
    print(f'\n{"="*60}')
    print(f'=== {name} ({size//1024} KB, at {path})')
    conn = sqlite3.connect(path)
    cur = conn.cursor()
    cur.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
    tables = cur.fetchall()
    for (tname,) in tables:
        cur.execute(f'SELECT sql FROM sqlite_master WHERE name="{tname}"')
        sql = cur.fetchone()
        print(f'\n  TABLE: {tname}')
        if sql: print(f'  Schema: {sql[0][:300]}')
        cur.execute(f'SELECT COUNT(*) FROM "{tname}"')
        cnt = cur.fetchone()[0]
        print(f'  Rows: {cnt}')
        if cnt > 0 and cnt < 50:
            cur.execute(f'SELECT * FROM "{tname}" LIMIT 5')
            cols = [d[0] for d in cur.description]
            print(f'  Sample: {cols}')
            for row in cur.fetchall():
                print(f'    {[str(v)[:60] for v in row]}')
        elif cnt > 0:
            # Show just column names and a summary
            cur.execute(f'SELECT * FROM "{tname}" LIMIT 2')
            cols = [d[0] for d in cur.description]
            print(f'  Columns ({len(cols)}): {cols}')
    conn.close()
