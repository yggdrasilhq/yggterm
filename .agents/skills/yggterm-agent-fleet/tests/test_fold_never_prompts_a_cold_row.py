#!/usr/bin/env python3
"""A stalled row is only ever prompted while it is still cheap to resume.

⛔ The standing fleet law: never kick a cold session. Cold cache times large
context multiply, the asking IS the expense, and prompting a cold row makes it
warm — so replacing it afterwards wastes exactly what the prompt just bought.

The law was already written, under "succeeding a session", framed as *do not ASK
a cold row what it was doing*. Someone implementing a WAKE path does not read
that as applying: a `continue` does not feel like asking. It is the same expense,
and a sweep sent one to rows carrying multi-megabyte transcripts.

⚠ Both conditions must hold. An OR would have the strength of the weaker test,
which is precisely how a five-megabyte row gets prompted for being recently idle.
"""
import importlib.util
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("fold", os.path.join(os.path.dirname(HERE), "ygg-fold.py"))
fold = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fold)


class OnlyACheapRowIsPrompted(unittest.TestCase):
    def test_a_small_recently_idle_row_may_be_woken(self):
        self.assertTrue(fold.wakeable(50_000, 8.0))

    def test_a_large_transcript_is_never_woken_however_recent(self):
        # The shape that broke the law: idle only a few minutes, but megabytes of
        # context that a prompt would force a full re-read of.
        self.assertFalse(fold.wakeable(5_000_000, 3.0))

    def test_a_small_row_gone_cold_is_never_woken(self):
        self.assertFalse(fold.wakeable(50_000, 120.0))

    def test_the_two_conditions_are_an_and_not_an_or(self):
        big, old = 5_000_000, 120.0
        small, fresh = 10_000, 1.0
        self.assertFalse(fold.wakeable(big, fresh))
        self.assertFalse(fold.wakeable(small, old))
        self.assertTrue(fold.wakeable(small, fresh))


if __name__ == "__main__":
    sys.exit(0 if unittest.main(exit=False).result.wasSuccessful() else 1)
