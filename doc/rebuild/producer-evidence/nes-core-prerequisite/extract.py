"""Recreate only this experiment's temporary exact JSNES dependency; no install."""
from pathlib import Path, PurePosixPath
import base64
import hashlib
import io
import json
import tarfile
import tempfile

root = Path(__file__).resolve().parent
state = json.loads((root / "preparation.json").read_text())
data = (root / "jsnes-2.1.0.tgz").read_bytes()
assert len(data) <= 4 * 1024 * 1024
assert "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode() == state["integrity"]
work = Path(tempfile.mkdtemp(prefix="vestra-jsnes-core-"))
total = 0
with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
    for member in archive:
        parts = PurePosixPath(member.name).parts
        assert parts and parts[0] == "package" and ".." not in parts
        if not member.isfile():
            continue
        if not (member.name.startswith("package/src/") or member.name in (
            "package/package.json", "package/LICENSE", "package/README.md", "package/index.d.ts"
        )):
            continue
        assert member.size <= 2 * 1024 * 1024
        total += member.size
        assert total <= 8 * 1024 * 1024
        content = archive.extractfile(member).read(2 * 1024 * 1024 + 1)
        assert len(content) == member.size
        target = work.joinpath(*parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
state["work"] = str(work)
(root / "preparation.json").write_text(json.dumps(state, indent=2) + "\n")
print(work)
