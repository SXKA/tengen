"""Compare Tengen's winning moves with the Gomocup judge's.

Usage: judge_fives.py POSITIONS RULE, where POSITIONS comes from
`cargo run -p tengen-arena --example dump_fives -- RULE N` and RULE is the
Piskvork number. GOMOCUP_JUDGE points at a checkout of
https://github.com/Gomocup/GomocupJudge. Exits nonzero on any mismatch.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.environ.get('GOMOCUP_JUDGE', 'GomocupJudge'), 'client'))
from check_forbid import check_forbid
rule = int(sys.argv[2])
N = 15
def judge_win(board, x, y, color):
    # GomocupJudge ai_match.make_move, without mutating the caller's board.
    if rule == 4 and color == 1 and check_forbid([r[:] for r in board], x, y):
        return False
    board[x][y] = color
    try:
        for nx, ny in [(0, 1), (1, 0), (-1, 1), (1, 1)]:
            c, blocked = 1, 0
            for sgn in (1, -1):
                _x, _y = x, y
                for i in range(1, 6):
                    _x += sgn * nx; _y += sgn * ny
                    if not (0 <= _x < N and 0 <= _y < N): break
                    if board[_x][_y] != color:
                        if board[_x][_y] != 0: blocked += 1
                        break
                    c += 1
            if rule in (0, 4) and c >= 5: return True
            if rule == 1 and c == 5: return True
            if rule == 9 and c == 5 and blocked < 2: return True
            if rule == 8 and (c > 5 or (c == 5 and blocked < 2)): return True
        return False
    finally:
        board[x][y] = 0
mismatch = wins = 0
for line in open(sys.argv[1]):
    stones, ours = line.split('|')
    board = [[0]*N for _ in range(N)]
    for s in stones.split():
        x, y, c = map(int, s.split(','))
        board[x][y] = c
    ours = set(tuple(map(int, s.split(','))) for s in ours.split())
    for x in range(N):
        for y in range(N):
            if board[x][y]: continue
            for color in (1, 2):
                theirs = judge_win(board, x, y, color)
                wins += theirs
                if theirs != ((x, y, color) in ours):
                    mismatch += 1
                    if mismatch <= 3: print('mismatch', (x, y, color), 'judge', theirs)
print(f'rule {rule}: {wins} winning moves per the judge, {mismatch} mismatches')
sys.exit(1 if mismatch else 0)
