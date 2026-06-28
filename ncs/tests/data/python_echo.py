#!/usr/bin/env python3
"""Echo tool — returns its arguments as output.
Used by NCS integration tests via !@Include.
"""
import sys
if len(sys.argv) > 1:
    print(" ".join(sys.argv[1:]))
else:
    print("(no args)")
