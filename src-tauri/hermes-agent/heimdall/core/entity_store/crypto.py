"""Device-local encryption/decryption (V2.2).

Uses Fernet (AES-128-CBC + HMAC-SHA256) with PBKDF2 key derivation.
Machine ID is stored in ~/.hermes/.machine_id; salt in ~/.hermes/.encryption_salt.
"""

import base64
import os
import uuid
from pathlib import Path

from cryptography.fernet import Fernet
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC


def _get_hermes_home() -> Path:
    home = Path.home() / ".hermes"
    home.mkdir(parents=True, exist_ok=True)
    return home


def _get_or_create_machine_id() -> str:
    path = _get_hermes_home() / ".machine_id"
    if not path.exists():
        machine_id = uuid.uuid4().hex
        path.write_text(machine_id)
    else:
        machine_id = path.read_text().strip()
    return machine_id


def _get_or_create_salt() -> bytes:
    path = _get_hermes_home() / ".encryption_salt"
    if not path.exists():
        salt = os.urandom(16)
        path.write_bytes(salt)
    else:
        salt = path.read_bytes()
    return salt


def _derive_key() -> bytes:
    machine_id = _get_or_create_machine_id().encode()
    salt = _get_or_create_salt()

    kdf = PBKDF2HMAC(
        algorithm=hashes.SHA256(),
        length=32,
        salt=salt,
        iterations=100000,
    )
    key = base64.urlsafe_b64encode(kdf.derive(machine_id))
    return key


_fernet = None


def _get_fernet() -> Fernet:
    global _fernet
    if _fernet is None:
        _fernet = Fernet(_derive_key())
    return _fernet


def device_encrypt(plaintext: str) -> bytes:
    return _get_fernet().encrypt(plaintext.encode())


def device_decrypt(ciphertext: bytes) -> str:
    return _get_fernet().decrypt(ciphertext).decode()
