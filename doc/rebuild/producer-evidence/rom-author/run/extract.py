
from pathlib import Path, PurePosixPath
import sys, tarfile, hashlib, json
archive, destination = Path(sys.argv[1]), Path(sys.argv[2])
assert hashlib.sha256(archive.read_bytes()).hexdigest() == '167081846a907e19bdd7cba75c648d827d49ea50e0fb58c079ad1a6613dee3c1'
seen, total, count = set(), 0, 0
with tarfile.open(archive, 'r:gz') as source:
 for member in source:
  path = PurePosixPath(member.name)
  assert not path.is_absolute() and path.parts and path.parts[0] == 'package' and '..' not in path.parts
  assert not member.issym() and not member.islnk() and not member.isdev()
  if member.isdir(): continue
  assert member.isfile()
  if not (member.name.startswith('package/src/') or member.name in ('package/package.json','package/LICENSE','package/README.md','package/index.d.ts')): continue
  assert member.name not in seen
  seen.add(member.name)
  assert 0 <= member.size <= 2*1024*1024
  total += member.size; count += 1
  assert total <= 8*1024*1024 and count <= 256
  data = source.extractfile(member).read(2*1024*1024+1)
  assert len(data) == member.size
  target = destination.joinpath(*path.parts)
  target.parent.mkdir(parents=True, exist_ok=True)
  with target.open('xb') as output: output.write(data)
metadata = json.loads((destination/'package/package.json').read_text())
assert metadata['name'] == 'jsnes' and metadata['version'] == '2.1.0'
print(json.dumps({'files': count, 'bytes': total, 'name': metadata['name'], 'version': metadata['version']}))
