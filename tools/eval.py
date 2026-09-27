#!/usr/bin/env python3
"""Typo benchmark for `kbdemo --query`: quality (top-1/3/8), latency, nodes, memory.

Usage: tools/eval.py DICT_FST [--types a,b,...] [--n 150] [--seed 1] [--bin PATH]
                        [--lexicon data/lexicon.tsv] [--words BIG_LIST.txt]

Target words are drawn by frequency rank from the lexicon. Typos that are
themselves words (in the lexicon, or in --words) are skipped — fixing those
needs sentence context. KB_* env vars pass through to kbdemo, e.g.
KB_PROFILE=phone.

Typo families:
  desktop model (same geometry as the desktop profile — optimistic):
    del sub_nb ins_nb trans double layout phonetic
  phone touch simulation (Gboard-like layout, independent of the engine model):
    touch_sub touch_off touch_ins touch_sub2 bounce omit_dbl short
  homerow: word typed without leaving the home row (touch-typing fingers)
"""
import argparse
import math
import os
import random
import re
import subprocess
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

ap = argparse.ArgumentParser()
ap.add_argument("dict")
ap.add_argument("--types", default="del,sub_nb,ins_nb,trans,double,layout,phonetic,"
                "touch_sub,touch_off,touch_ins,touch_sub2,bounce,omit_dbl,short,homerow")
ap.add_argument("--n", type=int, default=150, help="cases per type")
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--bin", default=f"{ROOT}/target/release/kbdemo")
ap.add_argument("--dump", default="", help="print failed cases of these types (comma-separated)")
ap.add_argument("--lexicon", default=f"{ROOT}/data/lexicon.tsv", help="word<TAB>count list for targets")
ap.add_argument("--words", help="extra word list: typos that are words in it are skipped too")
args = ap.parse_args()
rng = random.Random(args.seed)

# --- desktop geometry (matches the engine's desktop profile) ---
D_ROWS = {"lat": ["qwertyuiop", "asdfghjkl", "zxcvbnm"], "cyr": ["йцукенгшщзхъ", "фывапролджэ", "ячсмитьбю"]}
D_OFF = [0.5, 0.75, 1.25]
dpos = {c: (x + D_OFF[y], y) for rows in D_ROWS.values() for y, r in enumerate(rows) for x, c in enumerate(r)}


def alpha(c):
    return "lat" if c < "а" else "cyr"


def neighbors(c):
    if c not in dpos:
        return []
    x, y = dpos[c]
    return [d for r in D_ROWS[alpha(c)] for d in r
            if d != c and (dpos[d][0] - x) ** 2 + (dpos[d][1] - y) ** 2 <= 1.3]


PHI = {}
for rl, rc in zip(D_ROWS["lat"], D_ROWS["cyr"]):
    for a, b in zip(rl, rc):
        PHI[a], PHI[b] = b, a


def t_del(w):
    i = rng.randrange(len(w))
    return w[:i] + w[i + 1:]


def t_sub(w):
    idx = [i for i, c in enumerate(w) if neighbors(c)]
    if not idx:
        return None
    i = rng.choice(idx)
    return w[:i] + rng.choice(neighbors(w[i])) + w[i + 1:]


def t_ins(w):
    i = rng.randrange(len(w))
    return w[:i] + rng.choice(neighbors(w[i]) + [w[i]]) + w[i:]


def t_trans(w):
    idx = [i for i in range(len(w) - 1) if w[i] != w[i + 1]]
    if not idx:
        return None
    i = rng.choice(idx)
    return w[:i] + w[i + 1] + w[i] + w[i + 2:]


def t_double(w):
    a = rng.choice([t_del, t_sub, t_ins, t_trans])(w)
    if not a or len(a) < 3:
        return None
    return rng.choice([t_del, t_sub, t_ins, t_trans])(a)


def t_layout(w):
    return "".join(PHI.get(c, c) for c in w)


PHON = [("е", "и"), ("и", "е"), ("о", "а"), ("а", "о"), ("ё", "е"), ("тся", "ться"), ("ться", "тся"),
        ("нн", "н"), ("сс", "с"), ("ie", "ei"), ("ei", "ie"), ("ll", "l"), ("ss", "s"), ("ance", "ence")]


def t_phon(w):
    opts = [(a, b) for a, b in PHON if a in w]
    if not opts:
        return None
    a, b = rng.choice(opts)
    k = rng.choice([m.start() for m in re.finditer(re.escape(a), w)])
    return w[:k] + b + w[k + len(a):]


# --- phone geometry (Gboard-like), independent of the engine's model ---
PH = {  # rows, row x-offsets (key widths), row height / key width
    "lat": (["qwertyuiop", "asdfghjkl", "zxcvbnm"], [0.0, 0.5, 1.5], 1.35),
    "cyr": (["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"], [0.0, 0.0, 1.0], 1.35 * 11 / 10),
}


def key_at(a, x, y):
    rows, offs, h = PH[a]
    r = math.floor(y / h)
    if not 0 <= r < 3:
        return None
    i = math.floor(x - offs[r])
    return rows[r][i] if 0 <= i < len(rows[r]) else None


def noisy_hit(c, mu=(0.0, 0.0)):
    """Key hit by a Gaussian tap aimed at `c`, conditioned on missing `c`."""
    a = alpha(c)
    rows, offs, h = PH[a]
    for r, row in enumerate(rows):
        if c in row:
            cx, cy = offs[r] + row.index(c) + 0.5, (r + 0.5) * h
            break
    else:
        return None
    s = 0.30 * (11 / 10 if a == "cyr" else 1.0)  # same absolute spread; ЙЦУКЕН keys are narrower
    for _ in range(200):
        k = key_at(a, cx + rng.gauss(mu[0], s), cy + rng.gauss(mu[1], s))
        if k and k != c:
            return k
    return None


def t_touch_sub(w, mu=(0.0, 0.0)):
    i = rng.randrange(len(w))
    k = noisy_hit(w[i], mu)
    return w[:i] + k + w[i + 1:] if k else None


def t_touch_off(w):
    return t_touch_sub(w, mu=(0.15, 0.35))  # thumb systematically lands low/right


def t_touch_ins(w):
    i = rng.randrange(len(w))
    k = noisy_hit(w[i])
    if not k:
        return None
    return w[:i] + k + w[i:] if rng.random() < 0.5 else w[:i + 1] + k + w[i + 1:]


def t_touch_sub2(w):
    a = t_touch_sub(w)
    return t_touch_sub(a) if a else None


def t_bounce(w):
    i = rng.randrange(len(w))
    return w[:i + 1] + w[i] + w[i + 1:]


def t_omit_double(w):
    idx = [i for i in range(len(w) - 1) if w[i] == w[i + 1]]
    if not idx:
        return None
    i = rng.choice(idx)
    return w[:i] + w[i + 1:]


# --- home-row typing: each letter becomes its finger's home key ---
FINGER_HOME = {}
for rows, fingers, homes in (
    (D_ROWS["cyr"], [[0, 1, 2, 3, 3, 4, 4, 5, 6, 7, 7, 7], [0, 1, 2, 3, 3, 4, 4, 5, 6, 7, 7], [0, 1, 2, 3, 3, 4, 4, 5, 6]], "фываолдж"),
    (D_ROWS["lat"], [[0, 1, 2, 3, 3, 4, 4, 5, 6, 7], [0, 1, 2, 3, 3, 4, 4, 5, 6], [0, 1, 2, 3, 3, 4, 4]], "asdfjkl"),
):
    for r, row in enumerate(rows):
        for i, c in enumerate(row):
            f = fingers[r][i]
            if f < len(homes):
                FINGER_HOME[c] = homes[f]


def t_homerow(w):
    if any(c not in FINGER_HOME for c in w):
        return None
    return "".join(FINGER_HOME[c] for c in w)


TYPES = {
    "del": t_del, "sub_nb": t_sub, "ins_nb": t_ins, "trans": t_trans, "double": t_double,
    "layout": t_layout, "phonetic": t_phon,
    "touch_sub": t_touch_sub, "touch_off": t_touch_off, "touch_ins": t_touch_ins,
    "touch_sub2": t_touch_sub2, "bounce": t_bounce, "omit_dbl": t_omit_double,
    "short": t_touch_sub, "homerow": t_homerow,
}


lexicon = []
with open(args.lexicon, encoding="utf-8") as f:
    for line in f:
        w, c = line.rstrip("\n").split("\t")
        lexicon.append((w, int(c)))
by_rank = [w for w, _ in sorted(lexicon, key=lambda wc: -wc[1])]
ranked = {"ru": [w for w in by_rank if w[0] >= "а"], "en": [w for w in by_rank if w[0] < "а"]}


def band(lang, lo, hi, minlen, maxlen):
    return [w for w in ranked[lang][lo:hi] if minlen <= len(w) <= maxlen and re.fullmatch(r"[a-z]+|[а-яё]+", w)]


pools = {
    "main": band("ru", 200, 30000, 4, 30) + band("en", 200, 20000, 4, 30),
    "short": band("ru", 20, 30000, 3, 4) + band("en", 20, 20000, 3, 4),
}
known = {w for w, _ in lexicon}
if args.words:
    with open(args.words, encoding="utf-8") as f:
        known |= {line.strip() for line in f}


def in_dict(w):
    return w in known


cases = []  # (type, typo, target)
for name in args.types.split(","):
    fn = TYPES[name]
    pool = pools["short" if name == "short" else "main"]
    ru = [w for w in pool if w[0] >= "а"]
    en = [w for w in pool if w[0] < "а"]
    got = tries = 0
    while got < args.n and tries < args.n * 50:
        tries += 1
        w = rng.choice(ru if (rng.random() < 0.7 or name == "homerow") else en)
        t = fn(w)
        if not t or t == w or in_dict(t):
            continue
        cases.append((name, t, w))
        got += 1

peak = [0]


def run(batch):
    p = subprocess.Popen([args.bin, "--query"] + [c[1] for c in batch], stdout=subprocess.PIPE,
                         env=dict(os.environ, DICT_FST=args.dict), text=True)

    def sample():
        while p.poll() is None:
            try:
                with open(f"/proc/{p.pid}/smaps_rollup") as f:
                    d = {l.split(":")[0]: int(l.split()[1]) for l in f if l.split()[-1] == "kB"}
                peak[0] = max(peak[0], d.get("Rss", 0))
                run.last = d
            except (OSError, ValueError, IndexError):
                pass
            time.sleep(0.005)

    th = threading.Thread(target=sample)
    th.start()
    out, _ = p.communicate()
    th.join()
    return out


run.last = {}
res = {}
all_ms = []
for b in range(0, len(cases), 400):
    batch = cases[b:b + 400]
    blocks = [bl for bl in re.split(r"\n\n", run(batch).strip()) if bl]
    assert len(blocks) == len(batch), (len(blocks), len(batch))
    for (typ, typo, tgt), bl in zip(batch, blocks):
        m = re.search(r"suggest ([\d.]+) ms(?:, (\d+) nodes)? \+ complete ([\d.]+) ms", bl)
        ms, nodes = float(m.group(1)), int(m.group(2) or 0)
        corr = re.findall(r"^\s+[→ ]\s(\S+)\s+[-\d.]+$", bl.split("completions:")[0], re.M)
        r = res.setdefault(typ, {"n": 0, "t1": 0, "t3": 0, "t8": 0, "ms": [], "nodes": []})
        r["n"] += 1
        r["t1"] += corr[:1] == [tgt]
        r["t3"] += tgt in corr[:3]
        r["t8"] += tgt in corr[:8]
        if typ in args.dump.split(",") and corr[:1] != [tgt]:
            print(f"  {typ:10} {typo:14} want {tgt:14} got {', '.join(corr[:3])}")
        r["ms"].append(ms)
        r["nodes"].append(nodes)
        all_ms.append(ms)

prof = os.environ.get("KB_PROFILE", "desktop")
print(f"dict={os.path.basename(args.dict)} profile={prof} preset={os.environ.get('KB_PRESET', 'balanced')} "
      f"cases={len(cases)} bin={os.path.relpath(args.bin, ROOT) if args.bin.startswith(ROOT) else args.bin}")
print(f"{'type':11} {'n':>4} {'top1':>6} {'top3':>6} {'top8':>6} {'p50ms':>6} {'p95ms':>6} {'max':>6} {'nodes':>6}")
tot = {"n": 0, "t1": 0, "t3": 0, "t8": 0}
for name in args.types.split(","):
    r = res.get(name)
    if not r:
        continue
    ms = sorted(r["ms"])
    nodes = sum(r["nodes"]) / len(r["nodes"])
    print(f"{name:11} {r['n']:4} {100 * r['t1'] / r['n']:5.1f}% {100 * r['t3'] / r['n']:5.1f}% "
          f"{100 * r['t8'] / r['n']:5.1f}% {ms[len(ms) // 2]:6.2f} {ms[int(len(ms) * .95)]:6.2f} {ms[-1]:6.2f} {nodes:6.0f}")
    for k in tot:
        tot[k] += r[k]
print(f"{'ALL':11} {tot['n']:4} {100 * tot['t1'] / tot['n']:5.1f}% {100 * tot['t3'] / tot['n']:5.1f}% "
      f"{100 * tot['t8'] / tot['n']:5.1f}%  mean {sum(all_ms) / len(all_ms):.2f} ms")
d = run.last
print(f"memory: peak RSS {peak[0] / 1024:.1f} MB (file-backed {d.get('Pss_File', 0) / 1024:.1f} MB), "
      f"anonymous {d.get('Anonymous', 0) / 1024:.2f} MB")
