"""Raw byte spans must survive replacement decoding and Unicode separators."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import composer


class RawRecordOffsets(unittest.TestCase):
    def test_raw_record_grammar_preserves_both_detector_paths(self):
        fault = b"NameError: name 'widget' is not defined\n"
        prefixes = [b"\xff\xc2\x85\n", b"\xff\xe2\x80\xa8\r\n",
                    b"\xff\xe2\x80\xa9\r", b"\xff\x0b\x0c\x1c\x1d\x1e\n",
                    "caf\u00e9\u0085\u2028\u2029\n".encode()]
        for prefix in prefixes:
            with self.subTest(prefix=prefix), tempfile.TemporaryDirectory() as root:
                path = Path(root) / "invented.log"
                path.write_bytes(prefix + fault)
                generation, _, lineage = composer.source_identity(path)
                page = composer.read_page(path, 0, 4096)
                text, cursor, done, offsets = page
                self.assertEqual(page.raw, prefix + fault)
                self.assertEqual(cursor, len(prefix + fault))
                self.assertTrue(done)
                self.assertEqual(offsets, [0, len(prefix)])
                direct, _ = composer.extract(
                    "board", path, generation, lineage, text, 0, offsets)
                stateful, _, _ = composer.extract_page_with_state(
                    "board", path, generation, lineage, text, 0,
                    cmd_fps={}, line_offs=offsets)
                for observations in (direct, stateful):
                    # Match the actual signature family without assuming its
                    # exact public name; both paths must see precisely one.
                    signatures = [o for o in observations
                                  if o["detector_id"].startswith("sig-")]
                    self.assertEqual(len(signatures), 1, observations)
                    self.assertEqual(signatures[0]["span"],
                                     f"bytes@{len(prefix)}")


if __name__ == "__main__":
    unittest.main()
