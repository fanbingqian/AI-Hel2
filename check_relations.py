import sqlite3, os

db = os.path.expanduser('~/.hermes/heimdall/heimdall.db')
conn = sqlite3.connect(db)
c = conn.cursor()

print('===== AI-Hel 知识库实体关系全景分析 =====\n')

# === KR 系统 ===
c.execute('SELECT COUNT(*) FROM kr_entities')
kr_entities = c.fetchone()[0]
c.execute('SELECT COUNT(*) FROM kr_relations')
kr_relations = c.fetchone()[0]
print(f'KR 实体: {kr_entities}')
print(f'KR 关系/边: {kr_relations}')
print()

# Entity types
c.execute('SELECT types, COUNT(*) as cnt FROM kr_entities GROUP BY types ORDER BY cnt DESC')
print('--- 实体类型分布 ---')
for row in c.fetchall():
    print(f'  types={str(row[0])[:40]:40s} {row[1]}')
print()

# Relation types
c.execute('SELECT type, COUNT(*) as cnt FROM kr_relations GROUP BY type ORDER BY cnt DESC')
print('--- 关系类型分布 ---')
for row in c.fetchall():
    print(f'  {str(row[0]):40s} x{row[1]}')
print()

# Most connected entities
c.execute('''
SELECT e.name, e.types,
    (SELECT COUNT(*) FROM kr_relations WHERE source_id = e.entity_id) as out_cnt,
    (SELECT COUNT(*) FROM kr_relations WHERE target_id = e.entity_id) as in_cnt
FROM kr_entities e
ORDER BY out_cnt + in_cnt DESC
LIMIT 15
''')
print('--- 连接最多的实体 Top 15 ---')
for row in c.fetchall():
    total = row[2] + row[3]
    print(f'  [{str(row[1])[:20]:20s}] {str(row[0])[:45]:45s} 出={row[2]} 入={row[3]} 总计={total}')
print()

# Isolated entities
c.execute('SELECT COUNT(*) FROM kr_entities WHERE entity_id NOT IN (SELECT DISTINCT source_id FROM kr_relations UNION SELECT DISTINCT target_id FROM kr_relations)')
isolated = c.fetchone()[0]
print(f'孤立实体 (无关系): {isolated}')
print(f'已连接: {kr_entities - isolated} ({(kr_entities-isolated)/kr_entities*100:.1f}%)')
print()

# Relation direction
c.execute('SELECT direction, COUNT(*) FROM kr_relations GROUP BY direction')
print('--- 关系方向 ---')
for row in c.fetchall():
    print(f'  direction={str(row[0]):10s} {row[1]}')
print()

# Source -> target type matrix
c.execute('''
SELECT r.type as rel_type,
    COALESCE((SELECT types FROM kr_entities WHERE entity_id = r.source_id), '?') as src_types,
    COALESCE((SELECT types FROM kr_entities WHERE entity_id = r.target_id), '?') as tgt_types,
    COUNT(*) as cnt
FROM kr_relations r
GROUP BY rel_type, src_types, tgt_types
ORDER BY cnt DESC
LIMIT 15
''')
print('--- 关系类型 × 实体类型矩阵 Top 15 ---')
for row in c.fetchall():
    print(f'  {str(row[0]):35s}  [{str(row[1])[:15]:15s}] -> [{str(row[2])[:15]:15s}]  x{row[3]}')
print()

# Causal chains
c.execute('SELECT COUNT(*) FROM kr_causal_chains')
print(f'因果链: {c.fetchone()[0]}')
c.execute('SELECT COUNT(*) FROM kr_inferences')
print(f'推理/推断: {c.fetchone()[0]}')
print()

# === Heimdall 系统 ===
print('=== Heimdall 知识系统 ===')
c.execute('SELECT COUNT(*) FROM heimdall_knowledge_entries')
c.execute('SELECT COUNT(*) FROM heimdall_knowledge_edges')
print(f'知识条目: {c.fetchone()[0]} (实际上一行是 heimdall_knowledge_entries)')
# Re-fetch
c.execute('SELECT COUNT(*) FROM heimdall_knowledge_entries')
print(f'知识条目 (entries): {c.fetchone()[0]}')
c.execute('SELECT COUNT(*) FROM heimdall_knowledge_edges')
print(f'知识边 (edges): {c.fetchone()[0]}')
c.execute('SELECT COUNT(*) FROM heimdall_memory_edges')
print(f'记忆边 (memory_edges): {c.fetchone()[0]}')
c.execute('SELECT COUNT(*) FROM heimdall_social_graph')
print(f'社交图 (social_graph): {c.fetchone()[0]}')
c.execute('SELECT COUNT(*) FROM heimdall_entities')
print(f'heimdall_entities: {c.fetchone()[0]}')
print()

# Edge types
c.execute('SELECT relation_type, COUNT(*) FROM heimdall_knowledge_edges GROUP BY relation_type ORDER BY COUNT(*) DESC')
print('--- 知识边类型 ---')
for row in c.fetchall():
    print(f'  {str(row[0]):30s} x{row[1]}')

conn.close()
