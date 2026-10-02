#!/usr/bin/env python3
"""Normalise a scenario's golden artifacts for tools/vng-suite.sh.

  golden.py <guest-script> <artifact-dir>     # normalised text on stdout

Directives in the guest script, one per line:
  # golden: FILE...                      artifacts that make up the golden
  # mask: REGEX => REPL -- REASON        re.sub on every line, in order
  # drop: REGEX -- REASON                drop matching lines
  # golden-include: FILE                 more mask/drop lines (path relative
                                         to the including file)
Every mask and drop needs a reason. The same normalisation runs on the Xorg
run (the golden) and the yserver run, so a divergence is masked on both.
"""
import os
import re
import sys

# Server-assigned numbers that legitimately differ between servers.
GENERIC = [
    (re.compile(r"(opcode:?\s+)\d+"), r"\1<op>"),
    (re.compile(r"(Major opcode of failed request:\s+)\d+"), r"\1<op>"),
    (re.compile(r"(first (?:event|error)(?: base)?:?\s+)\d+"), r"\1<base>"),
]


def directives(script, files=None, rules=None):
    files = [] if files is None else files
    rules = [] if rules is None else rules
    for n, line in enumerate(open(script), 1):
        m = re.match(r"#\s*(golden|golden-include|mask|drop):\s*(.*)$", line.rstrip("\n"))
        if not m:
            continue
        kind, rest = m.groups()
        where = f"{script}:{n}"
        if kind == "golden":
            files += rest.split()
            continue
        if kind == "golden-include":
            directives(os.path.join(os.path.dirname(script), rest.strip()), files, rules)
            continue
        body, sep, reason = rest.rpartition(" -- ")
        if not sep or not reason.strip():
            sys.exit(f"golden.py: {where}: {kind} without a ' -- reason'")
        if kind == "mask":
            pat, sep, repl = body.partition(" => ")
            if not sep:
                sys.exit(f"golden.py: {where}: mask needs 'REGEX => REPL'")
            rules.append(("mask", re.compile(pat), repl))
        else:
            rules.append(("drop", re.compile(body), None))
    return files, rules


def main():
    script, art = sys.argv[1], sys.argv[2]
    files, rules = directives(script)
    if not files:
        sys.exit(f"golden.py: {script} has no '# golden:' line")
    out = []
    for f in files:
        out.append(f"==> {f} <==")
        try:
            lines = open(f"{art}/{f}", errors="replace").read().splitlines()
        except OSError:
            out.append("(missing)")
            continue
        for line in lines:
            for pat, repl in GENERIC:
                line = pat.sub(repl, line)
            dropped = False
            for kind, pat, repl in rules:
                if kind == "drop" and pat.search(line):
                    dropped = True
                    break
                if kind == "mask":
                    line = pat.sub(repl, line)
            if not dropped:
                out.append(line.rstrip())
    print("\n".join(out))


main()
