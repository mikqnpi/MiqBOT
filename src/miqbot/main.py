"""CLI entry point for MiqBOT."""

from __future__ import annotations

from argparse import ArgumentParser

from .bot import MiqBot


def main() -> None:
    parser = ArgumentParser(description="Run MiqBOT")
    parser.add_argument("--name", default="MiqBOT", help="Bot name")
    parser.add_argument("--message", default="Hello from MiqBOT")
    args = parser.parse_args()

    bot = MiqBot(name=args.name)
    bot.say(args.message)


if __name__ == "__main__":
    main()
