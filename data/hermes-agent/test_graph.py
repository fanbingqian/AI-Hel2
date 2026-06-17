import urllib.request, json

elist = json.loads(urllib.request.urlopen('http://localhost:8765/api/entities?limit=5').read())
for e in elist['entities']:
    name = e['display_name']
    eid = e['entity_id']
    g = json.loads(urllib.request.urlopen(f'http://localhost:8765/api/entities/{eid}/graph').read())
    print(name + ' (' + e['entity_type'] + '): nodes=' + str(len(g['nodes'])) + ' edges=' + str(len(g['edges'])))
    for ed in g['edges']:
        print('  ' + ed['from'] + ' -> ' + ed['to'] + ' type=' + ed.get('edge_type', '?'))
