"""Authentication service."""

from typing import Optional


def authenticate(username: str, password: str) -> bool:
    """Authenticate a user."""
    return validate_credentials(username, password)


def validate_credentials(username: str, password: str) -> bool:
    """Validate user credentials."""
    # Simplified validation
    return len(username) > 0 and len(password) >= 8


def get_user(user_id: int) -> Optional[dict]:
    """Get user by ID."""
    # Mock implementation
    return {"id": user_id, "name": "Test User"}


def hash_password(password: str) -> str:
    """Hash a password."""
    return f"hashed_{password}"
