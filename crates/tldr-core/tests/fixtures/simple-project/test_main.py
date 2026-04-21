"""Tests for main module."""

import pytest
from main import main, process_data, add_to_total


def test_main():
    """Test the main function."""
    result = main()
    assert result == 6


def test_process_data():
    """Test process_data function."""
    assert process_data([1, 2, 3]) == 6
    assert process_data([]) == 0


def test_add_to_total():
    """Test add_to_total function."""
    assert add_to_total(0, 5) == 5
    assert add_to_total(10, 3) == 13
