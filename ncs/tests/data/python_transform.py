#!/usr/bin/env python3
"""Transform tool — reads stdin and uppercases each line.
Used by NCS integration tests.
"""
import sys
for line in sys.stdin:
    print(line.strip().upper())
