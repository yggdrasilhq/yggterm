#!/usr/bin/env bash
# Execute the profiling notebooks headlessly and write the rendered output.
#
# Everything here runs from a shell on any fleet host: the notebooks are
# standard-library-only, so the venv holds nothing but the notebook machinery
# and nothing needs a GUI, a browser, or a scientific stack.
#
#   ./run.sh                 # all notebooks, 30m window, local host
#   ./run.sh 02 05           # only those two
#   YGG_NOTEBOOK_HOSTS=a,b YGG_NOTEBOOK_GUI_HOST=b ./run.sh
#   YGG_NOTEBOOK_WINDOW=2h ./run.sh
#
# Hosts are configuration and never literals in a tracked file. "local" means
# this machine; anything else is an ssh alias.
set -uo pipefail
cd "$(dirname "$0")"

VENV="${YGG_NOTEBOOK_VENV:-$HOME/.cache/yggterm-notebooks/venv}"
OUT="${YGG_NOTEBOOK_OUT:-out}"

if [ ! -x "$VENV/bin/jupyter" ]; then
  echo "bootstrapping notebook venv at $VENV"
  python3 -m venv "$VENV" || exit 1
  "$VENV/bin/pip" install --quiet --disable-pip-version-check \
    jupyter-core nbformat nbclient nbconvert ipykernel || exit 1
fi

mkdir -p "$OUT"
if [ "$#" -gt 0 ]; then
  books=(); for p in "$@"; do books+=( "$p"*.ipynb ); done
else
  books=( [0-9]*.ipynb )
fi

rc=0
for book in "${books[@]}"; do
  [ -e "$book" ] || { echo "no such notebook: $book" >&2; rc=1; continue; }
  echo "=== $book ==="
  # --allow-errors: a probe that cannot reach a host must not abort the run.
  # The verdict cell is what reports that, and it reports it as INSUFFICIENT
  # DATA rather than as a pass.
  "$VENV/bin/jupyter" nbconvert --to notebook --execute --allow-errors \
      --ExecutePreprocessor.timeout="${YGG_NOTEBOOK_TIMEOUT:-600}" \
      --output-dir "$OUT" --output "${book%.ipynb}.executed.ipynb" \
      "$book" >/dev/null 2>"$OUT/${book%.ipynb}.log" || { echo "  EXECUTION FAILED (see $OUT/${book%.ipynb}.log)"; rc=1; continue; }
  "$VENV/bin/python" render_verdicts.py "$OUT/${book%.ipynb}.executed.ipynb"
done
exit $rc
