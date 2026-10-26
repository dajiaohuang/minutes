#!/usr/bin/env python3
"""Reject Linux release images requiring a newer glibc than Ubuntu 24.04."""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("images", nargs="+", type=Path)
    args = parser.parse_args()
    maximum = (2, 39)
    failed = False
    for image in args.images:
        result = subprocess.run(
            ["readelf", "--version-info", "--wide", str(image)],
            check=True,
            capture_output=True,
            text=True,
        )
        versions = {
            tuple(map(int, version.split(".")))
            for version in re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", result.stdout)
        }
        newest = max(versions, default=())
        accepted = not newest or newest <= maximum
        failed = failed or not accepted
        print(json.dumps({
            "image": str(image),
            "maximum_allowed": "2.39",
            "maximum_required": ".".join(map(str, newest)) if newest else None,
            "accepted": accepted,
        }))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
