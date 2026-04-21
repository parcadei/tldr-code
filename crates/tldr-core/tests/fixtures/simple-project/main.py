"""Simple project main module."""


def main():
    """Entry point for the application."""
    result = process_data([1, 2, 3])
    print(f"Result: {result}")
    return result


def process_data(items: list) -> int:
    """Process a list of items and return the sum."""
    total = 0
    for item in items:
        total = add_to_total(total, item)
    return total


def add_to_total(current: int, value: int) -> int:
    """Add a value to the current total."""
    return current + value


def unused_function():
    """This function is never called - dead code."""
    pass


if __name__ == "__main__":
    main()
