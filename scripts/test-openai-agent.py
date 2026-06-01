#!/usr/bin/env python3
"""Send one OpenAI-compatible request through Agentland and print the result."""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ENV_FILE = ROOT / ".env.agentland-test"


def load_env(path: Path) -> None:
    if not path.exists():
        return

    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ.setdefault(key.strip(), value.strip().strip('"').strip("'"))


def request_json(method: str, url: str, *, headers: dict[str, str] | None = None, body: dict | None = None) -> tuple[int, str]:
    data = None
    request_headers = headers or {}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        request_headers = {"Content-Type": "application/json", **request_headers}

    req = urllib.request.Request(url, data=data, headers=request_headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            return response.status, response.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read().decode("utf-8", errors="replace")
    except urllib.error.URLError as exc:
        raise RuntimeError(f"Could not connect to {url}: {exc.reason}") from exc


def main() -> int:
    load_env(ENV_FILE)

    proxy_url = os.environ.get("AGENTLAND_PROXY_URL", "http://localhost:4000").rstrip("/")
    api_url = os.environ.get("AGENTLAND_API_URL", "http://localhost:4001").rstrip("/")
    api_key = os.environ.get("OPENAI_API_KEY", "")
    model = os.environ.get("OPENAI_MODEL", "gpt-4o-mini")
    agent_id = os.environ.get("AGENTLAND_AGENT_ID", "local-test-agent")
    message = os.environ.get("AGENTLAND_TEST_MESSAGE", "Reply with exactly: pong")

    if not api_key:
        print(f"OPENAI_API_KEY is missing. Set it in {ENV_FILE} or your shell.", file=sys.stderr)
        return 2

    print(f"Checking Agentland API: {api_url}/health")
    health_status, health_body = request_json("GET", f"{api_url}/health")
    print(f"health HTTP {health_status}: {health_body}")
    if health_status >= 400:
        return 1

    endpoint = f"{proxy_url}/proxy/openai/v1/chat/completions"
    payload = {
        "model": model,
        "messages": [{"role": "user", "content": message}],
        "max_tokens": 20,
    }
    headers = {
        "Authorization": f"Bearer {api_key}",
        "X-Agentland-Agent-Id": agent_id,
        "Agent-Name": "Local Test Agent",
    }

    print(f"\nSending agent request through: {endpoint}")
    status, body = request_json("POST", endpoint, headers=headers, body=payload)
    print(f"completion HTTP {status}:")
    print(pretty_json(body))

    events_url = f"{api_url}/api/v1/events?{urllib.parse.urlencode({'limit': 5})}"
    print(f"\nRecent Agentland events: {events_url}")
    events_status, events_body = request_json("GET", events_url)
    print(f"events HTTP {events_status}:")
    print(pretty_json(events_body))

    if status in {401, 403}:
        print("\nUpstream rejected the API key, but Agentland should still have captured the request event.")
    return 0 if events_status < 400 else 1


def pretty_json(text: str) -> str:
    try:
        return json.dumps(json.loads(text), indent=2)
    except json.JSONDecodeError:
        return text


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        raise SystemExit(1)
