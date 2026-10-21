"""The mock Google Analytics 4 of the SaaS gate (`SRC-3` V-GA4.4), registered into
`saas_stub.py`. It serves the Data API's REST calls at Google's real paths and Google's
token endpoint (`/token`, the path of `https://oauth2.googleapis.com/token`):

- `GET  /v1beta/properties/<id>/metadata`: the dimensions and metrics the property
  accepts (`apiName`, metrics with a `type`), plus the custom dimensions the gate adds;
- `POST /v1beta/properties/<id>:runReport`: `dimensions`, `metrics`, `dateRanges`, with
  `offset` and `limit` as TEXT (int64 in the JSON mapping), answering `dimensionHeaders`,
  `metricHeaders` (name and `type`), `rows` of `dimensionValues` / `metricValues` (text),
  and `rowCount` (the total). A date dimension is `YYYYMMDD`. A name the property does
  not have is 400 `INVALID_ARGUMENT`. Empty rows are not returned, as Google does;
- `POST /token`: the refresh-token grant (client in the form body), and the JWT-bearer
  grant of a service-account key. The assertion is checked for real: three parts, header
  RS256/JWT, claims `iss` (the service account), `scope`, `aud` = Google's token address,
  `exp` at most one hour after `iat`, and the RSASSA-PKCS1-v1_5 SHA-256 signature against
  the public key the gate registers (`register_key`, modulus and exponent) with stdlib
  integer arithmetic only. The answer is a bearer token that lives an hour.

Failures in Google's shape (`{"error": {"code", "message", "status"}}`): 401
UNAUTHENTICATED, 403 PERMISSION_DENIED, 429 RESOURCE_EXHAUSTED with `Retry-After`, and a
refused grant as 400 `invalid_grant`. Every error body carries `STUB-VENDOR-WORDS`.

Data: five days (yesterday back) of two countries, `sessions` (an integer) and
`purchaseRevenue` (a currency). Nothing is real; credentials are placeholders.
"""
from __future__ import annotations

import base64
import hashlib
import json
import time
from datetime import datetime, timedelta, timezone

import saas_stub as stub

CLIENT_ID = "stub-client"
CLIENT_SECRET = "stub-client-secret"
REFRESH_TOKEN = "stub-refresh-token"
BROWSER_TOKEN = "stub-ga-access"
SA_TOKEN = "stub-ga-sa-access"
SA_EMAIL = "reader@example.invalid"
TOKEN_ADDRESS = "https://oauth2.googleapis.com/token"
SCOPE = "https://www.googleapis.com/auth/analytics.readonly"
_SHA256_PREFIX = bytes.fromhex("3031300d060960864801650304020105000420")  # DER DigestInfo of SHA-256

_DIMENSIONS = ["date", "country", "dateHour", "yearMonth"]
_METRICS = {"sessions": "TYPE_INTEGER", "purchaseRevenue": "TYPE_CURRENCY", "activeUsers": "TYPE_INTEGER"}


def _day(n: int) -> str:
    """`YYYYMMDD` of the day `n` days before yesterday (UTC), as the mock's clock sees it."""
    return (datetime.now(timezone.utc).date() - timedelta(days=1 + n)).strftime("%Y%m%d")


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        self.rows = {(_day(n), c): {"sessions": 10 + n, "purchaseRevenue": float(n) + 0.5, "activeUsers": 5} for n in range(5) for c in ("ID", "US")}
        self.custom_dimensions: list[str] = []
        self.key: tuple[int, int] | None = None
        self.revoked = False
        self.forbidden = False
        self.rate_limits_left = 0
        self.log: list[str] = []


STATE = State()


def _error(handler, code: int, status: str, headers: dict | None = None) -> None:  # noqa: ANN001
    messages = {401: "Request had invalid authentication credentials", 403: "The caller does not have permission", 429: "Exhausted property tokens", 400: "Invalid argument"}
    stub.send_json(handler, {"error": {"code": code, "message": f"{messages[code]} STUB-VENDOR-WORDS", "status": status}}, code, headers)


def _authorised(handler, parsed, note: str = "") -> bool:  # noqa: ANN001
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}{note}")
        if STATE.revoked or handler.headers.get("Authorization") not in (f"Bearer {BROWSER_TOKEN}", f"Bearer {SA_TOKEN}"):
            _error(handler, 401, "UNAUTHENTICATED")
            return False
        if STATE.forbidden:
            _error(handler, 403, "PERMISSION_DENIED")
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            _error(handler, 429, "RESOURCE_EXHAUSTED", {"Retry-After": "1"})
            return False
    return True


def _metadata(handler, parsed) -> None:  # noqa: ANN001
    if not _authorised(handler, parsed):
        return
    with stub.STATE_LOCK:
        dims = [{"apiName": d, "uiName": d, "customDefinition": False} for d in _DIMENSIONS] + [{"apiName": d, "uiName": d, "customDefinition": True} for d in STATE.custom_dimensions]
        stub.send_json(handler, {"name": parsed.path.removeprefix("/v1beta/"), "dimensions": dims, "metrics": [{"apiName": m, "type": t} for m, t in _METRICS.items()]})


def _run_report(handler, parsed) -> None:  # noqa: ANN001
    body = stub.read_json(handler)
    ranges = body["dateRanges"][0]
    if not _authorised(handler, parsed, f" {ranges['startDate']}..{ranges['endDate']} offset={body.get('offset')}"):
        return
    wanted_dims = [d["name"] for d in body.get("dimensions", [])]
    wanted_metrics = [m["name"] for m in body.get("metrics", [])]
    with stub.STATE_LOCK:
        known = set(_DIMENSIONS) | set(STATE.custom_dimensions)
        if not set(wanted_dims) <= known or not set(wanted_metrics) <= set(_METRICS) or not isinstance(body.get("offset"), str) or not isinstance(body.get("limit"), str):
            return _error(handler, 400, "INVALID_ARGUMENT")
        lo, hi = ranges["startDate"].replace("-", ""), ranges["endDate"].replace("-", "")
        if lo > hi:
            return _error(handler, 400, "INVALID_ARGUMENT")
        hits = sorted((k, v) for k, v in STATE.rows.items() if lo <= k[0] <= hi)
        start, size = int(body["offset"]), int(body["limit"])
        rows = []
        for (day, country), metrics in hits[start : start + size]:
            values = {"date": day, "country": country}
            rows.append({
                "dimensionValues": [{"value": values.get(d, "x")} for d in wanted_dims],
                "metricValues": [{"value": str(metrics[m])} for m in wanted_metrics],
            })  # fmt: skip
        stub.send_json(handler, {
            "dimensionHeaders": [{"name": d} for d in wanted_dims],
            "metricHeaders": [{"name": m, "type": _METRICS[m]} for m in wanted_metrics],
            "rows": rows, "rowCount": len(hits), "kind": "analyticsData#runReport",
        })  # fmt: skip


def _properties(handler, parsed) -> None:  # noqa: ANN001
    stub.send_json(handler, {"error": {"code": 404, "message": "not found STUB-VENDOR-WORDS", "status": "NOT_FOUND"}}, 404)


def _get(handler, parsed) -> None:  # noqa: ANN001
    _metadata(handler, parsed) if parsed.path.endswith("/metadata") else _properties(handler, parsed)


def _post(handler, parsed) -> None:  # noqa: ANN001
    _run_report(handler, parsed) if parsed.path.endswith(":runReport") else _properties(handler, parsed)


def _unb64(part: str) -> bytes:
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


def _assertion_is_valid(assertion: str) -> bool:
    """Check a JWT-bearer assertion as Google does (structure, claims, RS256 signature)."""
    try:
        head, claims, sig = assertion.split(".")
        header, claim = json.loads(_unb64(head)), json.loads(_unb64(claims))
    except ValueError:
        return False
    if header != {"alg": "RS256", "typ": "JWT"} or STATE.key is None:
        return False
    if claim.get("iss") != SA_EMAIL or claim.get("scope") != SCOPE or claim.get("aud") != TOKEN_ADDRESS:
        return False
    if not (isinstance(claim.get("iat"), int) and isinstance(claim.get("exp"), int)) or not 0 < claim["exp"] - claim["iat"] <= 3600:
        return False
    if abs(time.time() - claim["iat"]) > 600:
        return False
    n, e = STATE.key
    size = (n.bit_length() + 7) // 8
    signature = _unb64(sig)
    if len(signature) != size:
        return False
    recovered = pow(int.from_bytes(signature, "big"), e, n).to_bytes(size, "big")
    digest = hashlib.sha256(f"{head}.{claims}".encode()).digest()
    expected = b"\x00\x01" + b"\xff" * (size - 3 - len(_SHA256_PREFIX) - 32) + b"\x00" + _SHA256_PREFIX + digest
    return recovered == expected


def _token(handler, parsed) -> None:  # noqa: ANN001
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        STATE.log.append(f"POST {parsed.path} {form.get('grant_type', '')}")
        grant = form.get("grant_type")
        token = None
        if not STATE.revoked:
            if grant == "refresh_token" and form.get("refresh_token") == REFRESH_TOKEN and form.get("client_id") == CLIENT_ID and form.get("client_secret") == CLIENT_SECRET:
                token = BROWSER_TOKEN
            elif grant == "urn:ietf:params:oauth:grant-type:jwt-bearer" and _assertion_is_valid(form.get("assertion", "")):
                token = SA_TOKEN
    if token is None:
        return stub.send_json(handler, {"error": "invalid_grant", "error_description": "Invalid JWT Signature. STUB-VENDOR-WORDS"}, 400)
    stub.send_json(handler, {"access_token": token, "expires_in": 3599, "token_type": "Bearer"})


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a Google route."""
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
        elif action == "register_key":  # the public half of the gate's throwaway service-account key
            STATE.key = (int(body["n"]), int(body["e"]))
        elif action == "put":  # Google corrects a day: `days_ago` counts back from yesterday
            STATE.rows[(_day(int(body["days_ago"])), body["country"])].update(body["metrics"])
        elif action == "add_dimension":
            STATE.custom_dimensions.append(body["name"])
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        elif action == "days":  # the dates the mock holds, for the gate's expectations
            return stub.send_json(handler, {"days": sorted({d for d, _c in STATE.rows})})
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", "/ga4/v1beta/properties")] = _get
stub.ROUTES[("POST", "/ga4/v1beta/properties")] = _post
stub.ROUTES[("POST", "/ga4/token")] = _token
stub.ROUTES[("GET", "/__admin/ga4")] = _admin
stub.ROUTES[("POST", "/__admin/ga4")] = _admin
