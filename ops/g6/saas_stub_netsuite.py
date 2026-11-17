"""The mock NetSuite of the SaaS gate (`SRC-3` V-NetSuite.4), registered into `saas_stub.py`. It
behaves as Oracle documents SuiteQL over REST and NetSuite's OAuth 2.0 for what the connector uses
(https://docs.oracle.com/en/cloud/saas/netsuite/ns-online-help/section_157909186990.html,
section_0312092247.html, section_160855590788.html); no SuiteQL function was run against a real
account, so the SQL it understands is only the shapes the connector writes:

- `POST /services/rest/query/v1/suiteql?limit=&offset=` with `Prefer: transient` (a request without
  it is refused), body `{"q": ...}`; the answer has `items` (each with a `links` array, a null column
  left out), `hasMore`, `count`, `offset`, `totalResults`; pages are capped at `STATE.page_cap`. A table or
  column it does not have is a 400. `lastmodifieddate` is answered in the US display form, the way the
  user's date format would show it, so the connector must not depend on it (it asks for `TO_CHAR`);
- `GET /services/rest/record/v1/employee?limit=1`: the connection test's request;
- `POST /services/rest/auth/oauth2/v1/token` (a form body): the refresh-token grant with the client in
  the Basic header, and the client-credentials grant with a signed assertion, checked for real: header
  `PS256` / `JWT` / `kid` = the certificate id, claims `iss`, `scope` = `rest_webservices`, `aud` = the
  token address, `iat`, `exp`, and the RSASSA-PSS signature against the public key the gate registers
  (`register_key`, modulus and exponent), with the standard library only;
- failures in NetSuite's shape (401, 403, 429 with `Retry-After`, 400); every error body carries
  `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

Credentials are placeholders that are only strings here.
"""
from __future__ import annotations

import base64
import hashlib
import json
import re
import time
from datetime import datetime, timedelta, timezone
from urllib.parse import parse_qs

import saas_stub as stub

ACCOUNT_HOST = "1234567-sb1.suitetalk.api.netsuite.com"
TOKEN_ADDRESS = f"https://{ACCOUNT_HOST}/services/rest/auth/oauth2/v1/token"
CLIENT_ID = "ns-client-id"
CERTIFICATE_ID = "cert-1"
OAUTH_CLIENT_ID = "stub-client"
OAUTH_CLIENT_SECRET = "stub-client-secret"
REFRESH_TOKEN = "stub-ns-refresh-0"
ACCESS_TOKEN = "ns-access"
SCOPE = "rest_webservices"
_STAMP = "%Y-%m-%d %H:%M:%S"


def _customer(i: int, moment: datetime, **extra) -> dict:
    iso = moment.astimezone(timezone.utc)
    return {"id": str(i), "companyname": f"customer {i}", "lastmodifieddate": f"{iso.month}/{iso.day}/{iso.year}", "_iso": iso.strftime(_STAMP), **extra}


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        now = datetime.now(timezone.utc)
        self.page_cap = 2
        self.tables: dict[str, list[dict]] = {
            "customer": [_customer(i, now - timedelta(days=13 - i)) for i in (1, 2, 3)],
            "item": [{"id": str(i), "itemid": f"SKU-{i}"} for i in (1, 2, 3)],  # no lastmodifieddate: read in full
        }
        self.key: tuple[int, int] | None = None
        self.revoked = False
        self.forbidden = False
        self.rate_limits_left = 0
        self.log: list[str] = []


STATE = State()


def _words(title: str, status: int) -> dict:
    return {"type": "https://www.rfc-editor.org/rfc/rfc9110.html", "title": title, "status": status, "o:errorDetails": [{"detail": "STUB-VENDOR-WORDS", "o:errorCode": "INVALID"}]}


def _gate(handler, parsed, what: str) -> bool:  # noqa: ANN001
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}?{parsed.query} {what}")
        if STATE.revoked or handler.headers.get("Authorization") != f"Bearer {ACCESS_TOKEN}":
            stub.send_json(handler, _words("Unauthorized", 401), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, _words("Forbidden", 403), 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, _words("Too Many Requests", 429), 429, {"Retry-After": "1"})
            return False
    return True


_SQL = re.compile(r"^SELECT (?P<cols>.+?) FROM (?P<table>[a-z_]+)(?: WHERE (?P<where>.+?))?(?: ORDER BY (?P<order>.+))?$")


def _suiteql(handler, parsed) -> None:  # noqa: ANN001
    body = stub.read_json(handler)
    sql = body.get("q", "") if isinstance(body, dict) else ""
    if not _gate(handler, parsed, sql):
        return
    query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
    if handler.headers.get("Prefer") != "transient":
        return stub.send_json(handler, _words("Prefer: transient is required", 400), 400)
    found = _SQL.match(sql)
    with stub.STATE_LOCK:
        rows = STATE.tables.get(found["table"]) if found else None
        if rows is None:
            return stub.send_json(handler, _words("Invalid table", 400), 400)
        cols = found["cols"]
        if cols in ("id, lastmodifieddate", "id") and any(c not in (rows[0] if rows else {"id": 1, "lastmodifieddate": 1}) for c in cols.split(", ")):
            return stub.send_json(handler, _words("Unknown identifier", 400), 400)
        table = found["table"]
        selected = list(rows)
        where = found["where"] or ""
        since = re.search(r"TO_TIMESTAMP\('([^']+)'", where)
        if since:
            selected = [r for r in selected if r["_iso"] >= since.group(1)]
        after = re.search(rf"{table}\.id > (\d+)", where)
        if after:
            selected = [r for r in selected if int(r["id"]) > int(after.group(1))]
        selected.sort(key=(lambda r: (r["_iso"], int(r["id"]))) if "lastmodifieddate" in (found["order"] or "") else (lambda r: int(r["id"])))
        offset, limit = int(query.get("offset", 0)), min(int(query.get("limit", 1000)), STATE.page_cap)
        page = selected[offset : offset + limit]
        items = []
        for row in page:
            item = {"links": [{"rel": "self", "href": f"https://{ACCOUNT_HOST}/services/rest/record/v1/{table}/{row['id']}"}], **{k: v for k, v in row.items() if k != "_iso"}}
            if "TO_CHAR" in cols and "_iso" in row:
                item["modified_at"] = row["_iso"]
            items.append(item)
    stub.send_json(handler, {"links": [], "count": len(items), "hasMore": offset + limit < len(selected), "items": items, "offset": offset, "totalResults": len(selected)})


def _whoami(handler, parsed) -> None:  # noqa: ANN001
    if _gate(handler, parsed, "whoami"):
        stub.send_json(handler, {"links": [], "count": 1, "hasMore": False, "items": [{"links": [], "id": "1"}], "offset": 0, "totalResults": 1})


# ---- the token endpoint ---------------------------------------------------------------------------------



def _unb64(part: str) -> bytes:
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


def _mgf1(seed: bytes, length: int) -> bytes:
    out, counter = b"", 0
    while len(out) < length:
        out += hashlib.sha256(seed + counter.to_bytes(4, "big")).digest()
        counter += 1
    return out[:length]


def _pss_verifies(n: int, e: int, signature: bytes, message: bytes) -> bool:
    """RSASSA-PSS (RFC 8017 9.1.2) with SHA-256, MGF1 and a salt of the hash's length: what `PS256` is."""
    mod_bits, h_len, s_len = n.bit_length(), 32, 32
    em_len = (mod_bits - 1 + 7) // 8
    if len(signature) != (mod_bits + 7) // 8:
        return False
    em = pow(int.from_bytes(signature, "big"), e, n).to_bytes((mod_bits + 7) // 8, "big")[-em_len:]
    if em_len < h_len + s_len + 2 or em[-1] != 0xBC:
        return False
    masked, digest = em[: em_len - h_len - 1], em[em_len - h_len - 1 : -1]
    zero_bits = 8 * em_len - (mod_bits - 1)
    if masked[0] >> (8 - zero_bits) if zero_bits else 0:
        return False
    block = bytearray(a ^ b for a, b in zip(masked, _mgf1(digest, len(masked))))
    if zero_bits:
        block[0] &= 0xFF >> zero_bits
    pad = em_len - h_len - s_len - 2
    if any(block[:pad]) or block[pad] != 1:
        return False
    salt = bytes(block[-s_len:])
    expected = hashlib.sha256(b"\x00" * 8 + hashlib.sha256(message).digest() + salt).digest()
    return expected == digest


def _assertion_is_valid(assertion: str) -> bool:
    """Check the assertion as NetSuite does (structure, header, claims, PS256 signature)."""
    try:
        head, claims, sig = assertion.split(".")
        header, claim = json.loads(_unb64(head)), json.loads(_unb64(claims))
    except ValueError:
        return False
    if header != {"alg": "PS256", "typ": "JWT", "kid": CERTIFICATE_ID} or STATE.key is None:
        return False
    if claim.get("iss") != CLIENT_ID or claim.get("scope") != SCOPE or claim.get("aud") != TOKEN_ADDRESS:
        return False
    if not (isinstance(claim.get("iat"), int) and isinstance(claim.get("exp"), int)) or not 0 < claim["exp"] - claim["iat"] <= 3600:
        return False
    if abs(time.time() - claim["iat"]) > 600:
        return False
    return _pss_verifies(STATE.key[0], STATE.key[1], _unb64(sig), f"{head}.{claims}".encode())


def _basic(user: str, password: str) -> str:
    return "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode()


def _token(handler, parsed) -> None:  # noqa: ANN001
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        grant = form.get("grant_type")
        STATE.log.append(f"POST {parsed.path} grant_type={grant}")
        ok = False
        if not STATE.revoked:
            if grant == "refresh_token":
                ok = (
                    handler.headers.get("Authorization") == _basic(OAUTH_CLIENT_ID, OAUTH_CLIENT_SECRET)
                    and "client_secret" not in form
                    and form.get("refresh_token") == REFRESH_TOKEN
                )
            elif grant == "client_credentials":
                ok = form.get("client_assertion_type") == "urn:ietf:params:oauth:client-assertion-type:jwt-bearer" and _assertion_is_valid(form.get("client_assertion", ""))
    if not ok:
        return stub.send_json(handler, {"error": "invalid_grant", "error_description": "Invalid assertion. STUB-VENDOR-WORDS"}, 400)
    stub.send_json(handler, {"access_token": ACCESS_TOKEN, "token_type": "bearer", "expires_in": 3600})


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a NetSuite route."""
    if handler.command == "GET":
        with stub.STATE_LOCK:
            return stub.send_json(handler, {"log": list(STATE.log)})
    body = stub.read_json(handler)
    action = body.get("action")
    with stub.STATE_LOCK:
        if action == "reset":
            STATE.reset()
        elif action == "clear_log":
            STATE.log.clear()
        elif action == "register_key":  # the public half of the gate's throwaway certificate key
            STATE.key = (int(body["n"]), int(body["e"]))
        elif action == "put":  # add or change a customer; NetSuite stamps lastmodifieddate itself
            fresh = _customer(int(body["id"]), datetime.now(timezone.utc), **body.get("fields", {}))
            STATE.tables["customer"] = [r for r in STATE.tables["customer"] if r["id"] != fresh["id"]] + [fresh]
        elif action == "put_item":
            STATE.tables["item"] = [r for r in STATE.tables["item"] if r["id"] != str(body["id"])] + [{"id": str(body["id"]), "itemid": body.get("itemid", "SKU")}]
        elif action == "add_field":
            STATE.tables["customer"] = [{**r, body["name"]: "x"} for r in STATE.tables["customer"]]
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("POST", "/netsuite/services/rest/query/v1/suiteql")] = _suiteql
stub.ROUTES[("GET", "/netsuite/services/rest/record/v1/employee")] = _whoami
stub.ROUTES[("POST", "/netsuite/services/rest/auth/oauth2/v1/token")] = _token
stub.ROUTES[("GET", "/__admin/netsuite")] = _admin
stub.ROUTES[("POST", "/__admin/netsuite")] = _admin
