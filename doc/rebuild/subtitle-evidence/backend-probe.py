"""Experimental standalone mpv subtitle probe; not player/API acceptance.

Creates its own fixtures and observes public CLI IPC. First-party Rust, the
plugin transport, and the software-rendered Slint canvas are not exercised.
"""
import json
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path


def main():
    work = Path(tempfile.mkdtemp(prefix="vestra-subtitle-cli-", dir="/tmp"))
    media, subtitle = work / "fixture.mkv", work / "fixture.srt"
    subtitle.write_text("1\n00:00:00,000 --> 00:00:07,500\nSUBTITLE PROBE\n\n")
    subprocess.run([
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
        "color=c=gray:s=640x360:r=30:d=8", "-f", "lavfi", "-i",
        "sine=frequency=440:duration=8", "-c:v", "libx264", "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-y", str(media),
    ], check=True)
    endpoint = work / "ipc"
    args = ["mpv", "--no-config", "--no-terminal", "--idle=yes", "--pause=yes",
            "--keep-open=yes", "--sub-auto=no", "--vo=null", "--ao=null",
            "--input-ipc-server=" + str(endpoint)]
    transcript, samples = [], {}
    with (work / "mpv.log").open("w") as log:
        process = subprocess.Popen(args, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 8
            while not endpoint.exists():
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("own mpv IPC did not become available")
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
                            raise RuntimeError("own mpv IPC closed unexpectedly")
                        received = json.loads(line)
                        transcript.append({"received": received})
                        if received.get("request_id") == sequence:
                            return received

                def get(name):
                    return request(["get_property", name])

                samples["load"] = request(["loadfile", str(media), "replace"])
                for _ in range(100):
                    duration = get("duration").get("data")
                    if isinstance(duration, (int, float)) and duration > 0:
                        break
                    time.sleep(.03)
                time.sleep(.2)
                samples["before"] = {name: get(name) for name in ["pause", "time-pos", "track-list", "sid"]}
                samples["beforeScreenshot"] = request(["screenshot-to-file", str(work / "before.png"), "subtitles"])
                samples["add"] = request(["sub-add", str(subtitle), "select", "probe"])
                time.sleep(.2)
                samples["selected"] = {name: get(name) for name in ["pause", "time-pos", "track-list", "sid"]}
                samples["selectedScreenshot"] = request(["screenshot-to-file", str(work / "selected.png"), "subtitles"])
                samples["off"] = request(["set_property", "sid", "no"])
                time.sleep(.1)
                samples["after"] = {name: get(name) for name in ["pause", "time-pos", "track-list", "sid"]}
                samples["offScreenshot"] = request(["screenshot-to-file", str(work / "off.png"), "subtitles"])
                samples["quit"] = request(["quit"])
            process.wait(timeout=5)
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
    frames = {}
    for name in ["before", "selected", "off"]:
        path = work / (name + ".png")
        if path.exists():
            frames[name] = subprocess.run([
                "ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(path),
                "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
            ], check=True, capture_output=True).stdout
    pixels = {}
    if len(frames) == 3:
        a, b, c = (frames[name] for name in ["before", "selected", "off"])
        assert len(a) == len(b) == len(c) == 640 * 360 * 3
        changed = [i // 3 for i in range(0, len(a), 3) if a[i:i+3] != b[i:i+3]]
        pixels = {
            "changedPixels": len(changed),
            "meanAbsDifference": sum(abs(x-y) for x, y in zip(a, b)) / len(a),
            "offEqualsBefore": a == c,
            "changedBounds": ([min(i % 640 for i in changed), min(i // 640 for i in changed),
                               max(i % 640 for i in changed), max(i // 640 for i in changed)]
                              if changed else None),
        }
    baseline_subs = [track for track in samples["before"]["track-list"].get("data", [])
                     if track["type"] == "sub"]
    checks = {
        "noBaselineSidecar": not baseline_subs,
        "pauseAndPositionUnchanged": all(samples[phase]["pause"].get("data") is True
                                        and samples[phase]["time-pos"].get("data") == 0
                                        for phase in ["before", "selected", "after"]),
        "selectedPixelsDiffer": pixels.get("changedPixels", 0) > 0,
        "offRestoresPixels": pixels.get("offEqualsBefore", False),
    }
    result = {"work": str(work), "command": args, "exitCode": process.returncode,
              "pass": all(checks.values()), "checks": checks, "pixels": pixels,
              "samples": samples,
              "limits": ["Standalone CLI only; no Rust/plugin/Slint path.",
                         "Screenshot command success is not pixel-change proof.",
                         "Null audio output does not prove audible continuity."]}
    (work / "result.json").write_text(json.dumps(result, indent=2))
    (work / "ipc.json").write_text(json.dumps(transcript, indent=2))
    destination = Path(__file__).parent / "backend-cli"
    destination.mkdir(exist_ok=True)
    for name in ["result.json", "ipc.json", "mpv.log", "fixture.srt", "fixture.mkv",
                 "before.png", "selected.png", "off.png"]:
        if (work / name).exists():
            shutil.copy2(work / name, destination / name)
    print(json.dumps({"work": str(work), "exitCode": process.returncode,
                      "pass": result["pass"], "checks": checks, "pixels": pixels,
                      "addResult": samples["add"],
                      "evidence": str(destination)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
