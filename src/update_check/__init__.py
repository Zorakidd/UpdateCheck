import sys

_USAGE = """usage: update-check [-h] [--check]

Weekly update checklist with reminders.

options:
  -h, --help  show this help message and exit
  --check     only open the window if this week's checklist is due (used by the
              automatic reminder)
"""


def main() -> None:
    # Parsed by hand rather than with argparse, which costs more to import than the whole rest of a
    # background check's start-up, for a single flag.
    args = sys.argv[1:]
    if "-h" in args or "--help" in args:
        print(_USAGE, end="")
        return
    if unknown := [arg for arg in args if arg != "--check"]:
        print(f"{_USAGE}\nunrecognized arguments: {' '.join(unknown)}", file=sys.stderr)
        raise SystemExit(2)

    from update_check.startup import run

    run(check_only="--check" in args)
