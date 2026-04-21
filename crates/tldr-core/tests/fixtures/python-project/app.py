"""Flask-like application with security-relevant patterns."""

from typing import Any
import os
import subprocess


# Simulated request object
class Request:
    args: dict = {"query": "SELECT * FROM users"}
    form: dict = {"password": "secret123"}
    json: dict = {"user_input": "test"}


request = Request()


def vulnerable_sql(user_input: str) -> None:
    """Vulnerable to SQL injection - for testing vuln detection."""
    query = f"SELECT * FROM users WHERE name = '{user_input}'"
    # cursor.execute(query)  # Would be vulnerable
    print(query)


def safe_sql(user_input: str) -> None:
    """Safe parameterized query."""
    query = "SELECT * FROM users WHERE name = ?"
    # cursor.execute(query, (user_input,))  # Safe
    print(query, user_input)


def command_injection_risk(filename: str) -> None:
    """Vulnerable to command injection - for testing."""
    os.system(f"cat {filename}")  # Dangerous!


def complex_function(a: int, b: int, c: int) -> int:
    """Function with high cyclomatic complexity for testing."""
    result = 0

    if a > 0:
        if b > 0:
            if c > 0:
                result = a + b + c
            else:
                result = a + b
        elif b < 0:
            result = a - b
        else:
            result = a
    elif a < 0:
        if b > 0:
            result = b - a
        else:
            result = -(a + b)
    else:
        if c != 0:
            result = c
        else:
            result = 0

    for i in range(10):
        if i % 2 == 0:
            result += i
        else:
            result -= i

    return result


def long_function():
    """A very long function for testing Long Method smell."""
    x = 1
    y = 2
    z = 3
    a = x + y
    b = y + z
    c = a + b
    d = c * 2
    e = d - 1
    f = e + 3
    g = f * 4
    h = g - 5
    i = h + 6
    j = i * 7
    k = j - 8
    l = k + 9
    m = l * 10
    n = m - 11
    o = n + 12
    p = o * 13
    q = p - 14
    r = q + 15
    s = r * 16
    t = s - 17
    u = t + 18
    v = u * 19
    w = v - 20
    return w


# Hardcoded secret for testing secret detection
API_KEY = "AKIAIOSFODNN7EXAMPLE"  # AWS-style key
PASSWORD = "super_secret_password_123"
