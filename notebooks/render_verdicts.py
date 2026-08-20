"""Print the text output of an executed notebook, verdict last.

The notebooks exist to be read by whoever or whatever is on the other end — the
owner, or the interface LLM reading a complaint. Neither of them should have to
open a notebook viewer to find out what the run concluded, so the runner prints
the cell output straight to the terminal.
"""
import json
import sys


def main(path: str) -> int:
    with open(path) as fh:
        book = json.load(fh)
    errors = 0
    for cell in book.get("cells", []):
        for out in cell.get("outputs", []):
            if out.get("output_type") == "stream":
                sys.stdout.write("".join(out.get("text", [])))
            elif out.get("output_type") == "error":
                errors += 1
                print(f"  !! {out.get('ename')}: {out.get('evalue')}")
    if errors:
        print(f"  ({errors} cell error(s) — the verdict above already accounts for missing data)")
    return 0


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        main(arg)
