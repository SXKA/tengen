"""Compare Tengen's renju forbidden points with the Gomocup judge's.

Usage: judge_forbid.py POSITIONS, where POSITIONS comes from
`cargo run -p tengen-arena --example dump_forbidden -- N`. GOMOCUP_JUDGE
points at a checkout of https://github.com/Gomocup/GomocupJudge. Exits
nonzero on any mismatch.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.environ.get('GOMOCUP_JUDGE', 'GomocupJudge'), 'client'))
from check_forbid import check_forbid
mismatch = total = flagged = 0
for line in open(sys.argv[1]):
    stones, ours = line.split('|')
    board = [[0]*15 for _ in range(15)]
    for s in stones.split():
        x, y, c = map(int, s.split(','))
        board[x][y] = c
    ours = set(tuple(map(int, s.split(','))) for s in ours.split())
    for x in range(15):
        for y in range(15):
            if board[x][y]: continue
            # The judge's five rule comes first in make_move; check_forbid is called before placing.
            theirs = bool(check_forbid([row[:] for row in board], x, y))
            total += 1
            flagged += theirs
            if theirs != ((x, y) in ours):
                mismatch += 1
                if mismatch <= 5:
                    print('mismatch at', (x, y), 'judge', theirs, 'tengen', (x, y) in ours)
                    print(stones.strip())
print(f'{total} cells, {flagged} forbidden per the judge, {mismatch} mismatches')
sys.exit(1 if mismatch else 0)
