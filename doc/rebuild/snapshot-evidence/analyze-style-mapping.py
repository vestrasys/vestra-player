"""Finite CLI image measurements; not public host/SDK subtitle acceptance."""
import json
import subprocess
from pathlib import Path


def decode(path):
    probe = subprocess.run(["ffprobe", "-v", "error", "-show_streams", "-of", "json", str(path)],
                           capture_output=True, check=True)
    stream = json.loads(probe.stdout)["streams"][0]
    width, height = stream["width"], stream["height"]
    decoded = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-frames:v", "1",
                              "-pix_fmt", "rgb24", "-f", "rawvideo", "-"], capture_output=True, check=True).stdout
    assert len(decoded) == width * height * 3
    return width, height, decoded


def main():
    for name in ["style-mapping-16", "style-mapping-72", "style-mapping-72-rgb"]:
        folder = Path(__file__).parent / name
        result, frames = {}, {}
        for renderer in ["literal", "offline"]:
            width, height, data = decode(folder / (renderer + ".png"))
            frames[renderer] = data
            points = [(i // 3 % width, i // 3 // width) for i in range(0, len(data), 3)
                      if data[i] > 160 and data[i + 1] > 160 and data[i + 2] < 100]
            result[renderer] = {"width": width, "height": height, "yellowPixels": len(points),
                                "bbox": [min(p[0] for p in points), min(p[1] for p in points),
                                         max(p[0] for p in points), max(p[1] for p in points)] if points else None}
        assert result["literal"]["width"] == result["offline"]["width"]
        assert result["literal"]["height"] == result["offline"]["height"]
        a, b = frames["literal"], frames["offline"]
        result["comparison"] = {"fullFrameMAE": sum(abs(x - y) for x, y in zip(a, b)) / len(a),
                                "exactDiffChannels": sum(x != y for x, y in zip(a, b)),
                                "maxAbsDiff": max(abs(x - y) for x, y in zip(a, b))}
        result["limits"] = ["Standalone CLI, not frozen host/SDK or physical display proof.",
                             "Yellow threshold is a measured color mask, not a glyph-content or font identity fence.",
                             "Full-frame MAE can dilute local glyph differences; no exact parity claim."]
        (folder / "mapping-analysis.json").write_text(json.dumps(result, indent=2))
        print(name, json.dumps(result))


if __name__ == "__main__":
    main()
