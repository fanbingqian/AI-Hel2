#!/usr/bin/env python3
"""
Gateway setup helper — JSON CLI wrapping QR registration for 5 platforms.

Usage:
  python gateway_setup_helper.py qr-start <platform>
  python gateway_setup_helper.py qr-poll <platform> <session_id>
  python gateway_setup_helper.py qr-cancel <platform> <session_id>

All output is JSON to stdout. Errors go to stderr.
Session state is stored in %TEMP%/ai-hel2-gateway-setup/ (or $TMPDIR on non-Windows).
"""

import json
import os
import sys
import time
import uuid
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

# Force UTF-8 output on Windows — otherwise Chinese characters get garbled
if sys.stdout.encoding != "utf-8":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if sys.stderr.encoding != "utf-8":
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

VERSION = "1.0.0"

# ── Session persistence ────────────────────────────────────────────────────

def _session_dir() -> Path:
    d = os.environ.get("TEMP") or os.environ.get("TMPDIR") or tempfile.gettempdir()
    p = Path(d) / "ai-hel2-gateway-setup"
    p.mkdir(parents=True, exist_ok=True)
    return p

def _load_session(session_id: str) -> dict:
    path = _session_dir() / f"{session_id}.json"
    if not path.exists():
        raise FileNotFoundError(f"Session {session_id} not found")
    return json.loads(path.read_text(encoding="utf-8"))

def _save_session(session_id: str, state: dict) -> None:
    path = _session_dir() / f"{session_id}.json"
    path.write_text(json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")

def _delete_session(session_id: str) -> None:
    path = _session_dir() / f"{session_id}.json"
    if path.exists():
        path.unlink()

def _generate_session_id() -> str:
    return uuid.uuid4().hex[:12]


# ── Output helpers ──────────────────────────────────────────────────────────

def _ok(data: dict) -> None:
    print(json.dumps({"ok": True, **data}, ensure_ascii=False))

def _err(message: str) -> None:
    print(json.dumps({"ok": False, "error": message}, ensure_ascii=False))
    sys.exit(1)


# ============================================================================
# WEIXIN (微信 iLink Bot)
# ============================================================================

def _weixin_qr_start() -> None:
    import urllib.request
    import urllib.error

    url = "https://ilinkai.weixin.qq.com/ilink/bot/get_bot_qrcode?bot_type=3"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "HermesAgent/1.0"})
        with urllib.request.urlopen(req, timeout=35) as resp:
            data = json.loads(resp.read().decode("utf-8"))
    except Exception as exc:
        _err(f"Weixin QR fetch failed: {exc}")

    qrcode_value = str(data.get("qrcode") or "")
    qrcode_url = str(data.get("qrcode_img_content") or "")
    if not qrcode_value:
        _err("Weixin QR response missing qrcode")

    session_id = _generate_session_id()
    state = {
        "platform": "weixin",
        "qrcode_value": qrcode_value,
        "qrcode_url": qrcode_url,
        "current_base_url": "https://ilinkai.weixin.qq.com",
        "refresh_count": 0,
        "created_at": time.time(),
    }
    _save_session(session_id, state)

    qr_data = qrcode_url if qrcode_url else qrcode_value
    _ok({
        "session_id": session_id,
        "qr_url": qr_data,
        "timeout_seconds": 480,
    })


def _weixin_qr_poll(session_id: str) -> None:
    import urllib.request
    import urllib.error

    state = _load_session(session_id)
    qrcode_value = state["qrcode_value"]
    qrcode_url = state["qrcode_url"]
    current_base_url = state["current_base_url"]
    refresh_count = state["refresh_count"]

    try:
        req = urllib.request.Request(
            f"{current_base_url}/ilink/bot/get_qrcode_status?qrcode={qrcode_value}",
            headers={"User-Agent": "HermesAgent/1.0"},
        )
        with urllib.request.urlopen(req, timeout=35) as resp:
            status_resp = json.loads(resp.read().decode("utf-8"))
    except Exception as exc:
        _err(f"Weixin QR poll failed: {exc}")

    status = str(status_resp.get("status") or "wait")

    if status == "confirmed":
        account_id = str(status_resp.get("ilink_bot_id") or "")
        token = str(status_resp.get("bot_token") or "")
        base_url = str(status_resp.get("baseurl") or "https://ilinkai.weixin.qq.com")
        user_id = str(status_resp.get("ilink_user_id") or "")
        if not account_id or not token:
            _err("Weixin QR confirmed but credentials incomplete")
        _delete_session(session_id)
        _ok({
            "status": "success",
            "credentials": {
                "account_id": account_id,
                "token": token,
                "base_url": base_url,
                "user_id": user_id,
            },
        })

    elif status == "scaned":
        _save_session(session_id, state)
        _ok({"status": "scanned", "message": "Scanned, waiting for confirmation..."})

    elif status == "scaned_but_redirect":
        redirect_host = str(status_resp.get("redirect_host") or "")
        if redirect_host:
            state["current_base_url"] = f"https://{redirect_host}"
        _save_session(session_id, state)
        _ok({"status": "waiting"})

    elif status == "expired":
        refresh_count += 1
        state["refresh_count"] = refresh_count
        if refresh_count > 3:
            _delete_session(session_id)
            _ok({"status": "expired", "message": "QR code expired too many times"})
            return
        # Try to refresh the QR code
        try:
            req = urllib.request.Request(
                "https://ilinkai.weixin.qq.com/ilink/bot/get_bot_qrcode?bot_type=3",
                headers={"User-Agent": "HermesAgent/1.0"},
            )
            with urllib.request.urlopen(req, timeout=35) as resp:
                data = json.loads(resp.read().decode("utf-8"))
            new_qrcode = str(data.get("qrcode") or "")
            new_qrcode_url = str(data.get("qrcode_img_content") or "")
            state["qrcode_value"] = new_qrcode
            state["qrcode_url"] = new_qrcode_url
            state["current_base_url"] = "https://ilinkai.weixin.qq.com"
            _save_session(session_id, state)
            qr_data = new_qrcode_url if new_qrcode_url else new_qrcode
            _ok({"status": "refreshed", "qr_url": qr_data, "refresh_count": refresh_count})
        except Exception:
            _delete_session(session_id)
            _ok({"status": "expired", "message": "QR expired, refresh failed"})

    else:
        _save_session(session_id, state)
        _ok({"status": "waiting"})


# ============================================================================
# WECOM (企业微信)
# ============================================================================

_WECOM_QR_GENERATE = "https://work.weixin.qq.com/ai/qc/generate"
_WECOM_QR_QUERY = "https://work.weixin.qq.com/ai/qc/query_result"


def _wecom_qr_start() -> None:
    import urllib.request
    import urllib.error

    url = f"{_WECOM_QR_GENERATE}?source=hermes"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "HermesAgent/1.0"})
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = json.loads(resp.read().decode("utf-8"))
    except Exception as exc:
        _err(f"WeCom QR fetch failed: {exc}")

    data = raw.get("data") or {}
    scode = str(data.get("scode") or "").strip()
    auth_url = str(data.get("auth_url") or "").strip()
    if not scode or not auth_url:
        _err("WeCom QR unexpected response format")

    session_id = _generate_session_id()
    state = {
        "platform": "wecom",
        "scode": scode,
        "created_at": time.time(),
    }
    _save_session(session_id, state)

    page_url = f"https://work.weixin.qq.com/ai/qc/gen?source=hermes&scode={urllib.parse.quote(scode)}"
    _ok({
        "session_id": session_id,
        "qr_url": auth_url,
        "page_url": page_url,
        "timeout_seconds": 300,
    })


def _wecom_qr_poll(session_id: str) -> None:
    import urllib.request
    import urllib.error

    state = _load_session(session_id)
    scode = state["scode"]

    url = f"{_WECOM_QR_QUERY}?scode={urllib.parse.quote(scode)}"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "HermesAgent/1.0"})
        with urllib.request.urlopen(req, timeout=10) as resp:
            result = json.loads(resp.read().decode("utf-8"))
    except Exception as exc:
        _ok({"status": "waiting", "error": str(exc)})
        return

    result_data = result.get("data") or {}
    status = str(result_data.get("status") or "").lower()

    if status == "success":
        bot_info = result_data.get("bot_info") or {}
        bot_id = str(bot_info.get("botid") or bot_info.get("bot_id") or "").strip()
        secret = str(bot_info.get("secret") or "").strip()
        if bot_id and secret:
            _delete_session(session_id)
            _ok({
                "status": "success",
                "credentials": {
                    "bot_id": bot_id,
                    "secret": secret,
                },
            })
        else:
            _delete_session(session_id)
            _ok({"status": "failed", "message": "Scan succeeded but no credentials returned"})
    else:
        _ok({"status": "waiting"})


# ============================================================================
# FEISHU (飞书 / Lark)
# ============================================================================

_FEISHU_ACCOUNTS_URLS = {
    "feishu": "https://accounts.feishu.cn",
    "lark": "https://accounts.larksuite.com",
}
_FEISHU_REGISTRATION_PATH = "/oauth/v1/app/registration"


def _feishu_post_registration(base_url: str, body: dict) -> dict:
    import urllib.request
    import urllib.error

    data = urllib.parse.urlencode(body).encode("utf-8")
    req = urllib.request.Request(
        f"{base_url}{_FEISHU_REGISTRATION_PATH}",
        data=data,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body_bytes = exc.read()
        if body_bytes:
            try:
                return json.loads(body_bytes.decode("utf-8"))
            except (ValueError, json.JSONDecodeError):
                raise
        raise


def _feishu_qr_start() -> None:
    domain = "feishu"
    base_url = _FEISHU_ACCOUNTS_URLS[domain]

    # Init
    try:
        res = _feishu_post_registration(base_url, {"action": "init"})
    except Exception as exc:
        _err(f"Feishu init failed: {exc}")

    methods = res.get("supported_auth_methods") or []
    if "client_secret" not in methods:
        _err(f"Feishu registration does not support client_secret auth. Supported: {methods}")

    # Begin
    try:
        res = _feishu_post_registration(base_url, {
            "action": "begin",
            "archetype": "PersonalAgent",
            "auth_method": "client_secret",
            "request_user_info": "open_id",
        })
    except Exception as exc:
        _err(f"Feishu begin registration failed: {exc}")

    device_code = res.get("device_code")
    if not device_code:
        _err("Feishu did not return a device_code")

    qr_url = res.get("verification_uri_complete", "")
    if "?" in qr_url:
        qr_url += "&from=hermes&tp=hermes"
    else:
        qr_url += "?from=hermes&tp=hermes"

    session_id = _generate_session_id()
    state = {
        "platform": "feishu",
        "device_code": device_code,
        "interval": res.get("interval") or 5,
        "expire_in": res.get("expire_in") or 600,
        "domain": domain,
        "domain_switched": False,
        "created_at": time.time(),
    }
    _save_session(session_id, state)

    _ok({
        "session_id": session_id,
        "qr_url": qr_url,
        "timeout_seconds": state["expire_in"],
        "user_code": res.get("user_code", ""),
    })


def _feishu_qr_poll(session_id: str) -> None:
    state = _load_session(session_id)
    device_code = state["device_code"]
    current_domain = state["domain"]

    base_url = _FEISHU_ACCOUNTS_URLS.get(current_domain, _FEISHU_ACCOUNTS_URLS["feishu"])
    try:
        res = _feishu_post_registration(base_url, {
            "action": "poll",
            "device_code": device_code,
            "tp": "ob_app",
        })
    except Exception:
        _ok({"status": "waiting"})
        return

    # Domain auto-detection
    user_info = res.get("user_info") or {}
    tenant_brand = user_info.get("tenant_brand")
    if tenant_brand == "lark" and not state.get("domain_switched"):
        state["domain"] = "lark"
        state["domain_switched"] = True
        _save_session(session_id, state)

    # Success
    if res.get("client_id") and res.get("client_secret"):
        _delete_session(session_id)
        _ok({
            "status": "success",
            "credentials": {
                "app_id": res["client_id"],
                "app_secret": res["client_secret"],
                "domain": state["domain"] if state.get("domain_switched") else "feishu",
                "open_id": user_info.get("open_id"),
            },
        })
        return

    # Terminal errors
    error = res.get("error", "")
    if error in {"access_denied", "expired_token"}:
        _delete_session(session_id)
        _ok({"status": "failed", "message": f"Registration {error}"})
        return

    _save_session(session_id, state)
    _ok({"status": "waiting"})


# ============================================================================
# QQBOT (QQ机器人)
# ============================================================================

_QQBOT_HOST = "q.qq.com"
_QQBOT_CREATE_PATH = "/lite/create_bind_task"
_QQBOT_POLL_PATH = "/lite/poll_bind_result"
_QQBOT_QR_TEMPLATE = "https://q.qq.com/lite/create_bot?task_id={task_id}"


def _qqbot_generate_aes_key() -> str:
    import base64
    return base64.b64encode(os.urandom(32)).decode()


def _qqbot_decrypt_secret(encrypted_base64: str, key_base64: str) -> str:
    import base64
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    key = base64.b64decode(key_base64)
    raw = base64.b64decode(encrypted_base64)
    iv = raw[:12]
    ciphertext_with_tag = raw[12:]
    aesgcm = AESGCM(key)
    plaintext = aesgcm.decrypt(iv, ciphertext_with_tag, None)
    return plaintext.decode("utf-8")


def _qqbot_qr_start() -> None:
    import urllib.request
    import urllib.error

    aes_key = _qqbot_generate_aes_key()

    url = f"https://{_QQBOT_HOST}{_QQBOT_CREATE_PATH}"
    try:
        data = json.dumps({"key": aes_key}).encode("utf-8")
        req = urllib.request.Request(
            url, data=data,
            headers={"Content-Type": "application/json", "Referer": f"https://{_QQBOT_HOST}/"}
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            resp_data = json.loads(resp.read().decode("utf-8"))
    except Exception as exc:
        _err(f"QQBot create bind task failed: {exc}")

    if resp_data.get("retcode") != 0:
        _err(f"QQBot create_bind_task error: {resp_data.get('msg', 'unknown')}")

    task_id = resp_data.get("data", {}).get("task_id")
    if not task_id:
        _err("QQBot missing task_id in response")

    qr_url = _QQBOT_QR_TEMPLATE.format(task_id=urllib.parse.quote(str(task_id)))

    session_id = _generate_session_id()
    state = {
        "platform": "qqbot",
        "task_id": task_id,
        "aes_key": aes_key,
        "refresh_count": 0,
        "created_at": time.time(),
    }
    _save_session(session_id, state)

    _ok({
        "session_id": session_id,
        "qr_url": qr_url,
        "timeout_seconds": 600,
    })


def _qqbot_qr_poll(session_id: str) -> None:
    import urllib.request
    import urllib.error

    state = _load_session(session_id)
    task_id = state["task_id"]
    aes_key = state["aes_key"]

    url = f"https://{_QQBOT_HOST}{_QQBOT_POLL_PATH}"
    try:
        data = json.dumps({"task_id": task_id}).encode("utf-8")
        req = urllib.request.Request(
            url, data=data,
            headers={"Content-Type": "application/json", "Referer": f"https://{_QQBOT_HOST}/"}
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            resp_data = json.loads(resp.read().decode("utf-8"))
    except Exception:
        _ok({"status": "waiting"})
        return

    if resp_data.get("retcode") != 0:
        _ok({"status": "waiting"})
        return

    d = resp_data.get("data", {})
    bind_status = d.get("status", 0)

    if bind_status == 2:  # COMPLETED
        app_id = str(d.get("bot_appid", ""))
        encrypted_secret = d.get("bot_encrypt_secret", "")
        user_openid = d.get("user_openid", "")

        if not app_id or not encrypted_secret:
            _err("QQBot completed but missing credentials")

        try:
            client_secret = _qqbot_decrypt_secret(encrypted_secret, aes_key)
        except Exception as exc:
            _err(f"QQBot secret decryption failed: {exc}")

        _delete_session(session_id)
        _ok({
            "status": "success",
            "credentials": {
                "app_id": app_id,
                "client_secret": client_secret,
                "user_openid": user_openid,
            },
        })

    elif bind_status == 3:  # EXPIRED
        state["refresh_count"] += 1
        if state["refresh_count"] > 3:
            _delete_session(session_id)
            _ok({"status": "expired", "message": "QR code expired too many times"})
            return
        # Auto-refresh: create new bind task
        new_aes_key = _qqbot_generate_aes_key()
        try:
            data = json.dumps({"key": new_aes_key}).encode("utf-8")
            req = urllib.request.Request(
                f"https://{_QQBOT_HOST}{_QQBOT_CREATE_PATH}",
                data=data,
                headers={"Content-Type": "application/json", "Referer": f"https://{_QQBOT_HOST}/"}
            )
            with urllib.request.urlopen(req, timeout=10) as resp:
                resp_data = json.loads(resp.read().decode("utf-8"))
            if resp_data.get("retcode") == 0:
                new_task_id = resp_data.get("data", {}).get("task_id")
                if new_task_id:
                    state["task_id"] = new_task_id
                    state["aes_key"] = new_aes_key
                    _save_session(session_id, state)
                    new_qr_url = _QQBOT_QR_TEMPLATE.format(task_id=urllib.parse.quote(str(new_task_id)))
                    _ok({"status": "refreshed", "qr_url": new_qr_url, "refresh_count": state["refresh_count"]})
                    return
        except Exception:
            pass
        _delete_session(session_id)
        _ok({"status": "expired", "message": "QR expired, refresh failed"})

    else:
        _ok({"status": "waiting"})


# ============================================================================
# DINGTALK (钉钉)
# ============================================================================

_DINGTALK_REGISTRATION_BASE = os.environ.get(
    "DINGTALK_REGISTRATION_BASE_URL", "https://oapi.dingtalk.com"
).rstrip("/")


def _dingtalk_api_post(path: str, payload: dict) -> dict:
    import urllib.request
    import urllib.error

    url = f"{_DINGTALK_REGISTRATION_BASE}{path}"
    try:
        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            url, data=data,
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=15) as resp:
            resp_data = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body = exc.read()
        if body:
            resp_data = json.loads(body.decode("utf-8"))
        else:
            raise

    errcode = resp_data.get("errcode", -1)
    if errcode != 0:
        errmsg = resp_data.get("errmsg", "unknown error")
        raise RuntimeError(f"DingTalk API error [{path}]: {errmsg} (errcode={errcode})")
    return resp_data


def _dingtalk_qr_start() -> None:
    try:
        init_data = _dingtalk_api_post("/app/registration/init", {"source": "openClaw"})
    except Exception as exc:
        _err(f"DingTalk init failed: {exc}")

    nonce = str(init_data.get("nonce", "")).strip()
    if not nonce:
        _err("DingTalk init response missing nonce")

    try:
        begin_data = _dingtalk_api_post("/app/registration/begin", {"nonce": nonce})
    except Exception as exc:
        _err(f"DingTalk begin registration failed: {exc}")

    device_code = str(begin_data.get("device_code", "")).strip()
    verification_uri = str(begin_data.get("verification_uri_complete", "")).strip()
    if not device_code:
        _err("DingTalk begin response missing device_code")
    if not verification_uri:
        _err("DingTalk begin response missing verification_uri_complete")

    expires_in = int(begin_data.get("expires_in", 7200))
    interval = max(int(begin_data.get("interval", 3)), 2)

    session_id = _generate_session_id()
    state = {
        "platform": "dingtalk",
        "device_code": device_code,
        "interval": interval,
        "expires_in": expires_in,
        "deadline_ts": time.time() + expires_in,
        "created_at": time.time(),
    }
    _save_session(session_id, state)

    _ok({
        "session_id": session_id,
        "qr_url": verification_uri,
        "timeout_seconds": expires_in,
    })


def _dingtalk_qr_poll(session_id: str) -> None:
    state = _load_session(session_id)
    device_code = state["device_code"]

    try:
        result = _dingtalk_api_post("/app/registration/poll", {"device_code": device_code})
    except Exception:
        _ok({"status": "waiting"})
        return

    status_raw = str(result.get("status", "")).strip().upper()
    if status_raw not in {"WAITING", "SUCCESS", "FAIL", "EXPIRED"}:
        status_raw = "UNKNOWN"

    if status_raw == "SUCCESS":
        client_id = str(result.get("client_id", "")).strip() or None
        client_secret = str(result.get("client_secret", "")).strip() or None
        if not client_id or not client_secret:
            _err("DingTalk succeeded but credentials missing")
        _delete_session(session_id)
        _ok({
            "status": "success",
            "credentials": {
                "client_id": client_id,
                "client_secret": client_secret,
            },
        })
    elif status_raw in {"FAIL", "EXPIRED"}:
        _delete_session(session_id)
        _ok({"status": "failed", "message": f"Authorization {status_raw.lower()}"})
    else:
        _ok({"status": "waiting"})


# ============================================================================
# QR-CANCEL (generic)
# ============================================================================

def _qr_cancel(session_id: str) -> None:
    try:
        _delete_session(session_id)
        _ok({"message": "Session cancelled"})
    except Exception as exc:
        _err(f"Cancel failed: {exc}")


# ============================================================================
# LIST PLATFORMS
# ============================================================================

def _list_platforms() -> None:
    _ok({
        "platforms": [
            {
                "key": "weixin",
                "label": "微信",
                "emoji": "",
                "description": "微信个人号 (iLink Bot)",
                "timeout_seconds": 480,
            },
            {
                "key": "wecom",
                "label": "企业微信",
                "emoji": "",
                "description": "企业微信 AI Bot",
                "timeout_seconds": 300,
            },
            {
                "key": "feishu",
                "label": "飞书",
                "emoji": "",
                "description": "飞书 / Lark 自建应用",
                "timeout_seconds": 600,
            },
            # QQBot QR API endpoints changed upstream (2025).
            # Old endpoints (/lite/create_bind_task, /lite/poll_bind_result)
            # now return retcode=12. QQBot requires manual configuration
            # via https://q.qq.com until the new API is documented.
            # {
            #     "key": "qqbot",
            #     "label": "QQ 机器人",
            #     "description": "QQ 开放平台机器人 (API 已变更，暂不支持扫码注册)",
            #     "timeout_seconds": 600,
            # },
            {
                "key": "dingtalk",
                "label": "钉钉",
                "emoji": "",
                "description": "钉钉机器人 (OAuth device flow)",
                "timeout_seconds": 7200,
            },
        ]
    })


# ============================================================================
# MAIN
# ============================================================================

_PLATFORM_HANDLERS = {
    "weixin": (_weixin_qr_start, _weixin_qr_poll),
    "wecom": (_wecom_qr_start, _wecom_qr_poll),
    "feishu": (_feishu_qr_start, _feishu_qr_poll),
    "dingtalk": (_dingtalk_qr_start, _dingtalk_qr_poll),
}


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"ok": False, "error": "Missing command. Use: qr-start, qr-poll, qr-cancel, list-platforms"}))
        sys.exit(1)

    cmd = sys.argv[1]

    if cmd == "list-platforms":
        _list_platforms()
        return

    if cmd == "qr-cancel":
        if len(sys.argv) < 3:
            _err("Usage: qr-cancel <session_id>")
        _qr_cancel(sys.argv[2])
        return

    if cmd == "qr-start":
        if len(sys.argv) < 3:
            _err("Usage: qr-start <platform>")
        platform = sys.argv[2]
        if platform not in _PLATFORM_HANDLERS:
            _err(f"Unknown platform: {platform}. Available: {list(_PLATFORM_HANDLERS.keys())}")
        start_fn, _ = _PLATFORM_HANDLERS[platform]
        start_fn()
        return

    if cmd == "qr-poll":
        if len(sys.argv) < 4:
            _err("Usage: qr-poll <platform> <session_id>")
        platform = sys.argv[2]
        session_id = sys.argv[3]
        if platform not in _PLATFORM_HANDLERS:
            _err(f"Unknown platform: {platform}")
        _, poll_fn = _PLATFORM_HANDLERS[platform]
        poll_fn(session_id)
        return

    _err(f"Unknown command: {cmd}. Use: qr-start, qr-poll, qr-cancel, list-platforms")


if __name__ == "__main__":
    main()
