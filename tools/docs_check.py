#!/usr/bin/env python3
"""Documentation gate: the reference docs must exist, be substantial, and stay
free of internal-only language."""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parents[1]
required = ["README.md", "AGENTS.md", "docs/PROTOCOL.md", "docs/LPF-FORMAT.md"]
banned = re.compile(r"\b(sigrok|decompil|disassembl|reverse[- ]?engineer|USBPcap)\b", re.I)

errors = []
for rel in required:
    path = root / rel
    if not path.is_file() or len(path.read_text().strip()) < 200:
        errors.append(f"missing or too short: {rel}")

for path in sorted((root / "docs").rglob("*.md")) + [root / "README.md", root / "AGENTS.md"]:
    if not path.is_file():
        continue
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if banned.search(line):
            errors.append(f"{path.relative_to(root)}:{number}: internal-only language")

if errors:
    print("\n".join(errors), file=sys.stderr)
    raise SystemExit(1)
print("docs check: ok")
