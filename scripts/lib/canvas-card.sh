#!/bin/bash
# A CANVAS TEST CARD — what a render-fault probe should have on screen.
#
# ⛔ AN EMPTY SHELL PROMPT PROVES ALMOST NOTHING. A capture of `user@host:~$` is
# a capture of about twelve glyphs on the default background, and the three open
# render faults need dense text and coloured cells before they have anywhere to
# appear. A probe that renders nothing interesting is a probe that always passes.
#
# The card therefore draws, deliberately, the exact conditions those faults were
# each observed under:
#   - a dense sweep of printable glyphs, so a SUBSTITUTION has somewhere to show
#   - coloured background runs, because the per-cell BLANKING was first seen
#     inside diff-coloured runs
#   - the same words on the DEFAULT background, because the blanking was later
#     seen there too, which is what refuted "the damage tracks background colour"
#
# The control lines repeat the words that were actually caught blanking, so a
# recurrence reads at a glance rather than needing a character count.
printf '== glyph coverage ==\n'
printf ' !"#$%%&()*+,-./0123456789:;<=>?@\n'
printf 'ABCDEFGHIJKLMNOPQRSTUVWXYZ[]^_\n'
printf 'abcdefghijklmnopqrstuvwxyz{|}~\n'
printf '== coloured background runs ==\n'
for _row in 1 2 3; do
  for colour in 1 2 3 4 5 6; do
    printf '\033[4%smABCDEFG abcdefg 0123456\033[0m' "$colour"
  done
  printf '\n'
done
printf '== default background control ==\n'
for _row in 1 2 3; do
  printf 'ABCDEFG abcdefg 0123456 shell commands another python3 read()\n'
done
printf '== card end ==\n'
