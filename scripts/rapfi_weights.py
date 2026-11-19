"""Convert Rapfi's classical freestyle model into a Tengen weights file.

Usage: python3 scripts/rapfi_weights.py MODEL.bin BASE_WEIGHTS.txt OUT.txt

MODEL.bin is a Rapfi classical model (LZ4 stream, e.g. model260809.bin from
https://github.com/dhbloo/rapfi); BASE_WEIGHTS.txt supplies the move-ordering
tables, which are kept. Needs the lz4 package. Rapfi's line patterns OL, B3S
and B4S have no Tengen counterpart; Tengen's patterns map to the Rapfi
pattern of the same name.
"""
import itertools
import struct
import sys

import lz4.frame

RAPFI = ["DEAD", "OL", "B1", "F1", "B2", "F2", "F2A", "F2B", "B3", "B3S", "F3", "F3S",
         "B4", "B4S", "F4", "F5"]
# Rapfi's freestyle blend of the current pattern value, the one before the last
# move and the threat term (eval/eval.cpp), and its value readout knots
# (value_readout_knots in config.toml), which the model file does not hold.
BLEND = [149, 106, 267]
READOUT = [0.0, 0.6655556577647762, 1.3147595695713945, 2.6539860186512536,
           4.91760799866031, 7.3644430081849706]
TENGEN = ["DEAD", "B1", "F1", "B2", "F2", "F2A", "F2B", "B3", "F3", "F3S", "B4", "F4", "F5"]


def codes(n):
    """Return {sorted 4-tuple: code} in lexicographic multiset order."""
    out = {}
    for t in itertools.combinations_with_replacement(range(n), 4):
        out[t] = len(out)
    return out


def main(model, base, out, scale_override=None):
    data = lz4.frame.decompress(open(model, "rb").read())
    sf = struct.unpack_from("<d", data, 0)[0]
    rc = codes(16)
    pn = len(rc)
    evals = struct.unpack_from(f"<{4 * pn}h", data, 8)
    off = 8 + 8 * pn
    threats = struct.unpack_from(f"<{4 * 2048}h", data, off)
    ev = evals[:pn]  # freestyle slot
    th = threats[:2048]
    k = 200.0 / sf if scale_override is None else scale_override
    tmap = [RAPFI.index(p) for p in TENGEN]
    tc = codes(13)
    pcode = [0] * len(tc)
    for t, c in tc.items():
        r = tuple(sorted(tmap[i] for i in t))
        pcode[c] = round(k * ev[rc[r]])
    threat = [0] * 8192
    for m in range(8192):
        b = lambda i: (m >> i) & 1
        r = (b(12) << 0 | b(0) << 1 | b(6) << 2 | (b(1) | b(2)) << 3 | b(3) << 4
             | b(4) << 5 | b(5) << 6 | (b(7) | b(8)) << 7 | b(9) << 8 | b(10) << 9
             | b(11) << 10)
        threat[m] = round(k * th[r])
    keep = [l for l in open(base).read().splitlines()
            if l.split(" ", 1)[0] in ("policy_own", "policy_opp", "policy_cross")]
    with open(out, "w") as f:
        f.write("# Freestyle evaluation converted from Rapfi's classical model\n")
        f.write("# (https://github.com/dhbloo/rapfi, GPL-3.0) by scripts/rapfi_weights.py,\n")
        f.write(f"# scaled by {k:.4f} from Rapfi's scaling factor {sf:.3f}.\n")
        f.write("pcode " + " ".join(map(str, pcode)) + "\n")
        f.write("pcode_late " + " ".join(map(str, pcode)) + "\n")
        f.write("threat " + " ".join(map(str, threat)) + "\n")
        for l in keep:
            f.write(l + "\n")
        f.write("blend " + " ".join(map(str, BLEND)) + "\n")
        f.write("readout " + " ".join(str(round(1000 * x)) for x in READOUT) + "\n")
    print("scale", sf, "k", k, "pcode range", min(pcode), max(pcode),
          "threat range", min(threat), max(threat))


if __name__ == "__main__":
    main(*sys.argv[1:4], *(float(x) for x in sys.argv[4:5]))
