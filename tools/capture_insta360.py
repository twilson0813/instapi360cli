"""
mitmproxy addon: capture Insta360 cloud API traffic from Insta360 Studio.

Run:
    mitmdump -s tools/capture_insta360.py --listen-port 8080

It prints, for every request to an insta360 API host, the method, URL, the
full request header set, and the response body (truncated). Auth/token header
VALUES are redacted so the log is safe to share. Matching flows are also saved
to tools/insta360_flows.txt for later inspection.

Goal: capture one successful `cloud/service/media/...` request so we can see
exactly which headers (especially any signature/nonce/timestamp/app-key) make
the server return media instead of 403 "no entitlement".
"""

from __future__ import annotations

import os
from mitmproxy import http

# Hosts we care about (Insta360 API surfaces).
INTEREST_HOST_SUBSTR = ("insta360.com",)
# Only log API-ish paths, skip static assets / analytics.
SKIP_PATH_SUBSTR = (
    "/_next/", "/static/", "sentry", "snssdk", "/collect",
    ".js", ".css", ".woff", ".svg", ".png", ".jpg", ".ico",
)
# Header names whose values are secrets — redact before printing/saving.
REDACT_HEADERS = {
    "authentication", "authorization", "x-user-token", "cookie",
    "set-cookie",
}

OUT_PATH = os.path.join(os.path.dirname(__file__), "insta360_flows.txt")


def _redact(name: str, value: str) -> str:
    if name.lower() in REDACT_HEADERS:
        n = len(value)
        if n <= 12:
            return "<redacted>"
        return f"{value[:6]}…{value[-4:]} ({n} chars)"
    return value


def _interesting(flow: http.HTTPFlow) -> bool:
    host = flow.request.pretty_host
    if not any(s in host for s in INTEREST_HOST_SUBSTR):
        return False
    path = flow.request.path
    if any(s in path for s in SKIP_PATH_SUBSTR):
        return False
    return True


def response(flow: http.HTTPFlow) -> None:
    if not _interesting(flow):
        return

    lines = []
    lines.append("=" * 80)
    lines.append(f"{flow.request.method} {flow.request.pretty_url}")
    lines.append("--- request headers ---")
    for name, value in flow.request.headers.items():
        lines.append(f"{name}: {_redact(name, value)}")

    if flow.request.method in ("POST", "PUT") and flow.request.content:
        body = flow.request.get_text(strict=False) or ""
        lines.append("--- request body ---")
        lines.append(body[:1000])

    status = flow.response.status_code if flow.response else "?"
    lines.append(f"--- response ({status}) ---")
    if flow.response:
        text = flow.response.get_text(strict=False) or ""
        lines.append(text[:1500])

    block = "\n".join(lines)
    print("\n" + block)

    with open(OUT_PATH, "a", encoding="utf-8") as fh:
        fh.write(block + "\n")
