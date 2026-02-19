"""Core MiqBOT implementation."""

class MiqBot:
    """Simple bot base class."""

    def __init__(self, name: str = "MiqBOT") -> None:
        self.name = name

    def say(self, message: str) -> None:
        """Output bot message."""
        print(f"[{self.name}] {message}")
