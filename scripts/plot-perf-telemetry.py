#!/usr/bin/env python3
import json
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path


def load_events(path: Path):
    events = []
    if not path.exists():
        return events
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return events


def duration_ms(event):
    payload = event.get("payload") or {}
    return payload.get("duration_ms")


def svg_escape(text: str) -> str:
    return (
        text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def main():
    if len(sys.argv) != 3:
        print("usage: plot-perf-telemetry.py <perf-telemetry.jsonl> <output.svg>", file=sys.stderr)
        return 1
    input_path = Path(sys.argv[1]).expanduser()
    output_path = Path(sys.argv[2]).expanduser()
    events = load_events(input_path)
    timed = [event for event in events if isinstance(duration_ms(event), (int, float))]
    if not timed:
        output_path.write_text(
            "<svg xmlns='http://www.w3.org/2000/svg' width='900' height='180'>"
            "<rect width='100%' height='100%' fill='#f8fbfd'/>"
            "<text x='32' y='72' font-size='24' font-family='sans-serif' fill='#24303a'>No perf telemetry yet</text>"
            "<text x='32' y='108' font-size='14' font-family='sans-serif' fill='#66727d'>Run yggterm once, then rerun this script.</text>"
            "</svg>",
            encoding="utf-8",
        )
        return 0

    grouped = defaultdict(list)
    for event in timed:
        grouped[f"{event.get('category','misc')}::{event.get('name','event')}"].append(duration_ms(event))

    summary = []
    for name, values in grouped.items():
        summary.append(
            {
                "name": name,
                "count": len(values),
                "avg": statistics.mean(values),
                "max": max(values),
                "p95": sorted(values)[max(0, math.ceil(len(values) * 0.95) - 1)],
            }
        )
    summary.sort(key=lambda item: item["max"], reverse=True)
    top = summary[:10]
    max_value = max(item["max"] for item in top) or 1.0

    width = 1180
    row_h = 34
    chart_x = 360
    chart_w = 760
    height = 110 + row_h * len(top)
    parts = [
        f"<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}'>",
        "<rect width='100%' height='100%' fill='#f6fafc'/>",
        "<text x='28' y='36' font-size='24' font-family='sans-serif' fill='#24303a'>Yggterm Performance Telemetry</text>",
        f"<text x='28' y='64' font-size='13' font-family='sans-serif' fill='#66727d'>Source: {svg_escape(str(input_path))}</text>",
        "<text x='28' y='86' font-size='13' font-family='sans-serif' fill='#66727d'>Sorted by max duration (ms)</text>",
    ]

    for index, item in enumerate(top):
        y = 120 + index * row_h
        bar_w = (item["max"] / max_value) * chart_w
        parts.append(
            f"<text x='28' y='{y}' font-size='13' font-family='sans-serif' fill='#24303a'>{svg_escape(item['name'])}</text>"
        )
        parts.append(
            f"<text x='28' y='{y + 16}' font-size='11' font-family='sans-serif' fill='#66727d'>avg {item['avg']:.1f} ms · p95 {item['p95']:.1f} ms · n={item['count']}</text>"
        )
        parts.append(
            f"<rect x='{chart_x}' y='{y - 14}' width='{chart_w}' height='16' rx='8' fill='rgba(207,221,232,0.68)'/>"
        )
        parts.append(
            f"<rect x='{chart_x}' y='{y - 14}' width='{bar_w:.1f}' height='16' rx='8' fill='#5b98ff'/>"
        )
        parts.append(
            f"<text x='{chart_x + chart_w + 12}' y='{y}' font-size='12' font-family='sans-serif' fill='#24303a'>{item['max']:.1f}</text>"
        )

    parts.append("</svg>")
    output_path.write_text("\n".join(parts), encoding="utf-8")
    print(output_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
