"""Group Rust test panics from a libtest log by root-cause signature."""
import collections
import re
import sys

text = open(sys.argv[1], encoding="utf-8-sig", errors="replace").read().splitlines()
panics = []
for i, line in enumerate(text):
    m = re.match(r"^thread '(.+?)' .*panicked at (.+?):\d+:\d+:$", line)
    if m:
        msg = " ".join(text[i + 1:i + 4])
        panics.append((m.group(1), m.group(2), msg))

RULES = [
    ("E87 confined rename via SetFileInformationByHandle+RootDirectory", r"os error 87|parameter is incorrect|code: 87"),
    ("E32 sharing violation (delete/rename of open file)", r"os error 32|being used by another process|code: 32"),
    ("E33 lock violation", r"os error 33|locked a portion"),
    ("private profile ACL", r"another user or group"),
    ("E5 access denied", r"os error 5\b|code: 5,|Access is denied"),
    ("E2 not found", r"os error 2\b|code: 2,|cannot find the file"),
]
groups = collections.defaultdict(list)
for test, where, msg in panics:
    for name, rx in RULES:
        if re.search(rx, msg):
            groups[name].append(test)
            break
    else:
        groups["other: " + where + " | " + re.sub(r"\s+", " ", msg)[:120]].append(test)

print(f"panics: {len(panics)}")
for name, tests in sorted(groups.items(), key=lambda kv: -len(kv[1])):
    print(f"{len(tests):4} {name}")
    if len(sys.argv) > 2:
        for t in tests[:4]:
            print(f"        {t}")
