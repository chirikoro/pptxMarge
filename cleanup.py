#!/usr/bin/env python3
"""Post-process merged PPTX to normalize XML structure.
Called by pptx-marge after Rust merge to fix any XML ordering issues.
Usage: python3 cleanup.py input.pptx output.pptx
"""
import sys
from pptx import Presentation

if len(sys.argv) != 3:
    print(f"Usage: {sys.argv[0]} input.pptx output.pptx", file=sys.stderr)
    sys.exit(1)

try:
    prs = Presentation(sys.argv[1])
    prs.save(sys.argv[2])
except Exception as e:
    print(f"Error: {e}", file=sys.stderr)
    sys.exit(1)
