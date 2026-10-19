"""Experimental standalone CLI flags/text/style probe, not player acceptance."""
import hashlib
import html
import json
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path


def main():
    destination = Path(__file__).parent / "backend-flags"
    work = Path(tempfile.mkdtemp(prefix="vestra-subtitle-flags-", dir="/tmp"))
    media = Path(__file__).parent / "backend-cli/fixture.mkv"
    literal = r"literal <b>B</b> {\b1}X{\b0} \N END"
    escaped = html.escape(literal, quote=False).replace("\\", "&#92;").replace("{", "&#123;").replace("}", "&#125;")
    for name, text in [("one", "PLAIN ONE"), ("two", "PLAIN TWO"),
                       ("literal", literal), ("entities", escaped)]:
        (work / (name + ".srt")).write_text("1\n00:00:00,000 --> 00:00:07,500\n" + text + "\n\n")
    endpoint = work / "ipc"
    args = ["mpv", "--no-config", "--no-terminal", "--idle=yes", "--pause=yes",
            "--keep-open=yes", "--sub-auto=no", "--vo=null", "--ao=null",
            "--input-ipc-server=" + str(endpoint)]
    transcript, observations = [], {}
    with (work / "mpv.log").open("w") as log:
        process = subprocess.Popen(args, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 8
            while not endpoint.exists():
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("own probe IPC did not become available")
                time.sleep(.02)
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(5)
                client.connect(str(endpoint))
                stream = client.makefile("rb")
                sequence = 0

                def request(command):
                    nonlocal sequence
                    sequence += 1
                    sent = {"command": command, "request_id": sequence}
                    transcript.append({"sent": sent})
                    client.sendall((json.dumps(sent) + "\n").encode())
                    while True:
                        line = stream.readline()
                        if not line:
                            raise RuntimeError("own probe IPC closed unexpectedly")
                        received = json.loads(line)
                        transcript.append({"received": received})
                        if received.get("request_id") == sequence:
                            return received

                def get(name):
                    return request(["get_property", name])

                def state():
                    return {name: get(name) for name in ["pause", "time-pos", "sid", "track-list", "sub-text", "sub-text-ass"]}

                def shot(name):
                    time.sleep(.15)
                    return request(["screenshot-to-file", str(work / (name + ".png")), "subtitles"])

                observations["load"] = request(["loadfile", str(media.resolve()), "replace"])
                for _ in range(100):
                    if isinstance(get("duration").get("data"), (int, float)):
                        break
                    time.sleep(.02)
                observations["baselineShot"] = shot("baseline")
                for name, flag in [("one", "auto"), ("two", "cached"), ("one", "cached")]:
                    key = name + "-" + flag
                    observations[key] = {"add": request(["sub-add", str(work / (name + ".srt")), flag]), "state": state()}
                    observations[key]["off"] = request(["set_property", "sid", "no"])
                for name in ["literal", "entities"]:
                    observations[name] = {"add": request(["sub-add", str(work / (name + ".srt")), "select"])}
                    observations[name]["shot"] = shot(name)
                    observations[name]["state"] = state()
                observations["style"] = {
                    "fontSize": request(["set_property", "sub-font-size", 80]),
                    "color": request(["set_property", "sub-color", "#FF0000"]),
                    "shot": shot("style"),
                    "state": state(),
                }
                observations["delay"] = {
                    "set": request(["set_property", "sub-delay", 3]),
                    "shot": shot("delay"),
                    "state": state(),
                }
                observations["resetDelay"] = {
                    "set": request(["set_property", "sub-delay", 0]),
                    "shot": shot("reset-delay"),
                    "state": state(),
                }
                observations["quit"] = request(["quit"])
            process.wait(timeout=5)
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
    frames = {}
    for name in ["baseline", "literal", "entities", "style", "delay", "reset-delay"]:
        path = work / (name + ".png")
        if path.exists():
            frames[name] = subprocess.run([
                "ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(path),
                "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
            ], check=True, capture_output=True).stdout
    comparisons = {}
    for left, right in [("baseline", "literal"), ("literal", "entities"),
                        ("entities", "style"), ("baseline", "delay"), ("style", "reset-delay")]:
        a, b = frames.get(left), frames.get(right)
        if a is not None and b is not None:
            assert len(a) == len(b) == 640 * 360 * 3
            comparisons[left + ":" + right] = {
                "equal": a == b,
                "changedPixels": sum(a[i:i+3] != b[i:i+3] for i in range(0, len(a), 3)),
                "meanAbsDifference": sum(abs(x-y) for x, y in zip(a, b)) / len(a),
            }
    result = {"work": str(work), "command": args, "exitCode": process.returncode,
              "literal": literal, "entityEncodedSrtText": escaped,
              "observations": observations, "pixelComparisons": comparisons,
              "decodedPixelHashes": {name: hashlib.sha256(data).hexdigest() for name, data in frames.items()},
              "limits": ["Standalone CLI screenshots only, not host/SDK/Slint acceptance.",
                         "Text properties do not alone prove visual escaping semantics.",
                         "Null audio output, no physical audio continuity proof."]}
    (work / "result.json").write_text(json.dumps(result, indent=2))
    (work / "ipc.json").write_text(json.dumps(transcript, indent=2))
    destination.mkdir(exist_ok=True)
    for path in work.iterdir():
        if path.is_file():
            shutil.copy2(path, destination / path.name)
    print(json.dumps({"work": str(work), "exitCode": process.returncode,
                      "flags": {name: {"add": observations[name]["add"], "sid": observations[name]["state"]["sid"]}
                                for name in ["one-auto", "two-cached", "one-cached"]},
                      "text": {name: {key: observations[name]["state"][key]
                                       for key in ["sub-text", "sub-text-ass"]}
                               for name in ["literal", "entities"]},
                      "pixels": comparisons}, ensure_ascii=False))


if __name__ == "__main__":
    main()
