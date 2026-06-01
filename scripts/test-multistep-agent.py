#!/usr/bin/env python3
"""Run a cheap multi-step OpenAI-compatible agent through Agentland."""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
ENV_FILE = ROOT / ".env.agentland-test"


TARGET_NUMBER = 17
MAX_AGENT_TURNS = 6

DIVIDE_NUMBER_TOOL = {
    "type": "function",
    "function": {
        "name": "divide_number",
        "description": "Divide one integer by one or more candidate divisors and report quotient, remainder, and divisibility.",
        "parameters": {
            "type": "object",
            "properties": {
                "number": {
                    "type": "integer",
                    "description": "The integer being tested.",
                },
                "divisors": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "Candidate divisors to test.",
                },
            },
            "required": ["number", "divisors"],
            "additionalProperties": False,
        },
    },
}


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


def extract_message(response_body: str) -> dict[str, Any]:
    try:
        data = json.loads(response_body)
        return data["choices"][0]["message"]
    except Exception:
        return {}


def extract_text(response_body: str) -> str:
    message = extract_message(response_body)
    content = message.get("content")
    return content.strip() if isinstance(content, str) else ""


def divide_number(number: int, divisor: int) -> dict[str, int | bool]:
    if divisor == 0:
        raise ValueError("divisor must not be zero")
    quotient, remainder = divmod(number, divisor)
    return {
        "number": number,
        "divisor": divisor,
        "quotient": quotient,
        "remainder": remainder,
        "divides_evenly": remainder == 0,
    }


def execute_tool_call(tool_call: dict[str, Any]) -> dict[str, str]:
    function = tool_call.get("function") or {}
    name = function.get("name")
    if name != "divide_number":
        content = json.dumps({"error": f"Unsupported tool: {name}"}, separators=(",", ":"))
    else:
        try:
            args = json.loads(function.get("arguments") or "{}")
            number = int(args["number"])
            divisors = [int(divisor) for divisor in args["divisors"]]
            content = json.dumps(
                {"results": [divide_number(number, divisor) for divisor in divisors]},
                separators=(",", ":"),
            )
        except Exception as exc:
            content = json.dumps({"error": str(exc)}, separators=(",", ":"))

    return {
        "role": "tool",
        "tool_call_id": str(tool_call.get("id", "")),
        "name": str(name or ""),
        "content": content,
    }


def initial_transcript(number: int) -> list[dict[str, Any]]:
    return [
        {
            "role": "system",
            "content": (
                "You are a tiny deterministic demo agent. Your job is to solve the user's goal with the tools available. "
                "Use divide_number when arithmetic validation is useful. Keep reasoning concise. "
                "When finished, return JSON with keys number, is_prime, reason, then a one-sentence answer."
            ),
        },
        {
            "role": "user",
            "content": f"Please determine whether {number} is prime.",
        },
    ]


def main() -> int:
    load_env(ENV_FILE)

    proxy_url = os.environ.get("AGENTLAND_PROXY_URL", "http://localhost:4000").rstrip("/")
    api_url = os.environ.get("AGENTLAND_API_URL", "http://localhost:4001").rstrip("/")
    api_key = os.environ.get("OPENAI_API_KEY", "")
    model = os.environ.get("OPENAI_MODEL", "gpt-4o-mini")
    run_id = str(int(time.time()))
    agent_id = os.environ.get("AGENTLAND_MULTI_AGENT_ID", f"prime-checker-demo-{run_id}")

    if not api_key:
        print(f"OPENAI_API_KEY is missing. Set it in {ENV_FILE} or your shell.", file=sys.stderr)
        return 2

    health_status, health_body = request_json("GET", f"{api_url}/health")
    print(f"Agentland health HTTP {health_status}: {health_body}")
    if health_status >= 400:
        return 1

    endpoint = f"{proxy_url}/proxy/openai/v1/chat/completions"
    headers = {
        "Authorization": f"Bearer {api_key}",
        "X-Agentland-Agent-Id": agent_id,
        "Agent-Name": "Prime Checker Demo Agent",
    }

    transcript = initial_transcript(TARGET_NUMBER)

    print(f"\nRunning agent goal as agent_id={agent_id}")
    print(f"Goal: {transcript[-1]['content']}")
    for i in range(1, MAX_AGENT_TURNS + 1):
        payload = {
            "model": model,
            "messages": transcript,
            "temperature": 0,
            "max_tokens": 120,
            "tools": [DIVIDE_NUMBER_TOOL],
            "tool_choice": "auto",
        }

        status, body = request_json("POST", endpoint, headers=headers, body=payload)
        print(f"\nAgent turn {i} HTTP {status}")
        print(pretty_json(body))
        if status >= 400:
            print("Stopping after upstream error; Agentland should still have logged this step.")
            break
        message = extract_message(body)
        tool_calls = message.get("tool_calls") or []
        if tool_calls:
            transcript.append(
                {
                    "role": "assistant",
                    "content": message.get("content"),
                    "tool_calls": tool_calls,
                }
            )
            for tool_call in tool_calls:
                tool_result = execute_tool_call(tool_call)
                print("\nLocal tool result:")
                print(pretty_json(tool_result["content"]))
                transcript.append(tool_result)
            time.sleep(0.2)
            continue
        answer = extract_text(body)
        transcript.append({"role": "assistant", "content": answer})
        print("\nAgent finished without requesting another tool.")
        break
    else:
        print(f"\nStopped after {MAX_AGENT_TURNS} agent turns to avoid an infinite loop.")

    if not any(message.get("role") == "tool" for message in transcript):
        print("\nNote: the model did not request a tool call on this run.")

    events_url = f"{api_url}/api/v1/reviews/trajectories?{urllib.parse.urlencode({'limit': 5})}"
    status, body = request_json("GET", events_url)
    print(f"\nReview queue HTTP {status}:")
    print(pretty_json(body))
    print("\nOpen http://localhost:3000/review and select the newest Prime Checker Demo Agent trajectory.")
    return 0 if status < 400 else 1


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
