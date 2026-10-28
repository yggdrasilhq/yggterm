import time
from pathlib import Path
import unittest

from .. import collectors, model


class StagedKernelLayout(unittest.TestCase):
    def test_ledger_path_dirs_cover_repo_and_staged_layouts(self):
        here = Path(collectors.__file__).resolve()
        dirs = collectors._ledger_path_dirs()
        # repo tree: tools/ (parents[1] of introspection/)
        self.assertIn(here.parents[1], dirs)
        # staged kernel: .../share/ygg/kernel/swarmharness and flat introspection/
        self.assertIn(here.parents[1] / "swarmharness", dirs)
        self.assertIn(here.parents[2], dirs)


class LiveComposerStateReadOnly(unittest.TestCase):
    """Reads the host composer state read-only when present; asserts only
    the metadata contract, never fleet facts. Skips when absent."""

    def test_live_composer_rows_are_metadata_only(self):
        obs = collectors.COMPOSER_STATE / "observations.jsonl"
        if not obs.exists():
            self.skipTest("no live composer state on this host")
        now = time.time()
        rows, state, detail = collectors.collect_composer(now)
        self.assertIn(state, ("ok", "absent", "stale"))
        for r in rows:
            model.validate_frame(model.frame(model.digest({"t": 1}), model.iso(now),
                                             [], {"composer": [r], "summary": []}))


if __name__ == "__main__":
    unittest.main()
