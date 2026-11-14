"""Hold stdin open for `rp2350-cli repl --stdio`.

`repl --stdio` bridges the host's stdin to the guest's USB CDC link and stops
once stdin reaches EOF. Firmware that sleeps for a while before printing (the
GBA example waits 1500 ms, i.e. hundreds of millions of emulated instructions)
would therefore never show its log. Pipe this script into the emulator to keep
the link alive:

    py example/gba/tools/hold_stdin.py 600 | rp2350-cli.exe repl --file emerald.uf2 --stdio --max-cycles 0

Usage: py hold_stdin.py [seconds]   (default 300; Ctrl-C stops early)
"""

import sys
import time


def main() -> int:
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 300.0
    try:
        time.sleep(seconds)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
