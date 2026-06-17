import json, sys, urllib.request
data = json.loads(urllib.request.urlopen("http://localhost:8765/api/entities?limit=8").read())
for e in data["entities"]:
    print(f'{e["display_name"]} ({e["entity_type"]}) count={e["occurrence_count"]}')
