import urllib.request, json

for pkg in ['langgraph', 'langchain', 'langchain-core']:
    try:
        d = json.load(urllib.request.urlopen(f'https://pypi.org/pypi/{pkg}/json'))
        v = d['info']['version']
        rels = d['releases']
        versions_13 = [x for x in rels if x.startswith('1.3')]
        times = {x: rels[x][0]['upload_time'][:10] for x in versions_13 if rels[x]}
        print(pkg, 'latest:', v)
        print('  1.3.x versions:', versions_13)
        print('  upload times:', times)
    except Exception as e:
        print(pkg, 'ERROR', repr(e))
