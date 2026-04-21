"""Utility functions."""

from typing import List, Optional


def helper_function(x: int) -> int:
    """A helper function."""
    return x * 2


def validate_input(data: Optional[List[int]]) -> bool:
    """Validate input data."""
    if data is None:
        return False
    if len(data) == 0:
        return False
    return all(isinstance(x, int) for x in data)


class DataProcessor:
    """Process data with various transformations."""

    def __init__(self, name: str):
        self.name = name
        self.data = []

    def add(self, item: int) -> None:
        """Add an item to the processor."""
        self.data.append(item)

    def process(self) -> int:
        """Process all data and return result."""
        return sum(self.data)

    def clear(self) -> None:
        """Clear all data."""
        self.data = []
