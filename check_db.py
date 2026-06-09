import sqlite3, os
db = os.path.expanduser('~/.hermes/heimdall/heimdall.db')
print(f"Checking DB at: {db}")
print(f"Exists: {os.path.exists(db)}")

conn = sqlite3.connect(db)
c = conn.cursor()
c.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
tables = [row[0] for row in c.fetchall()]
print(f"\nTables ({len(tables)}):")
for t in tables:
    print(f"  - {t}")

# Schema for each
for tbl in tables:
    c.execute(f"SELECT sql FROM sqlite_master WHERE name='{tbl}'")
    sql = c.fetchone()[0]
    # Show first line
    line = sql[:150].replace('\n', ' | ')
    print(f"\n  {tbl}: {line}...")

conn.close()
