"""HEIMDALL Privacy Utilities — salted hashing for third-party identities.

SHA-256(name + per-user 128-bit salt) for de-identification.
Salt is stored in the system keychain/file and never leaves the device.
"""

from __future__ import annotations

import hashlib
import os
import secrets
from pathlib import Path


def generate_salt() -> str:
    """Generate a 128-bit (16-byte) random salt, hex-encoded."""
    return secrets.token_hex(16)


def load_or_create_salt(salt_path: Path) -> str:
    """Load salt from file, or generate and persist a new one.

    Args:
        salt_path: Path to the salt file (e.g., ~/.heimdall/.salt).

    Returns:
        Hex-encoded 128-bit salt string.
    """
    if salt_path.exists():
        return salt_path.read_text().strip()
    salt = generate_salt()
    salt_path.parent.mkdir(parents=True, exist_ok=True)
    salt_path.write_text(salt)
    _set_restrictive_perms(salt_path)
    return salt


def hash_name(name: str, salt: str) -> str:
    """Hash a display name with the user's salt.

    SHA-256(name + salt) -> 64 hex characters.
    The original name is NOT recoverable from the hash.
    """
    if not name or not salt:
        return ""
    return hashlib.sha256(f"{name}{salt}".encode("utf-8")).hexdigest()


def is_third_party(entity_type: str) -> bool:
    """Determine if an entity type represents a third-party identity.

    Only 'person' type entities get salted-hash de-identification.
    The user themselves (entity_type='self') keeps plaintext display_name.
    """
    return entity_type == "person"


def _set_restrictive_perms(path: Path) -> None:
    """Set 0600 permissions on the salt file (Unix only)."""
    if os.name != "nt":
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass
