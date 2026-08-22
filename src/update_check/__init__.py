import argparse


def main() -> None:
    parser = argparse.ArgumentParser(prog="update-check", description="Weekly update checklist with reminders.")
    parser.add_argument(
        "--check",
        action="store_true",
        help="only open the window if this week's checklist is due (used by the automatic reminder)",
    )
    args = parser.parse_args()

    from update_check.app import run

    run(check_only=args.check)
