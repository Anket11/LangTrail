from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "scripts" / "test-multistep-agent.py"
SPEC = importlib.util.spec_from_file_location("test_multistep_agent", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
agent = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(agent)


class MultistepAgentToolTests(unittest.TestCase):
    def test_initial_transcript_has_one_goal_without_scripted_steps(self) -> None:
        transcript = agent.initial_transcript(17)

        user_messages = [message for message in transcript if message["role"] == "user"]
        self.assertEqual(len(user_messages), 1)
        self.assertNotIn("Step 1", user_messages[0]["content"])
        self.assertIn("determine whether 17 is prime", user_messages[0]["content"])

    def test_divide_number_reports_remainder_and_divisibility(self) -> None:
        result = agent.divide_number(17, 3)

        self.assertEqual(
            result,
            {
                "number": 17,
                "divisor": 3,
                "quotient": 5,
                "remainder": 2,
                "divides_evenly": False,
            },
        )

    def test_execute_tool_call_runs_model_requested_divisions(self) -> None:
        tool_call = {
            "id": "call_123",
            "type": "function",
            "function": {
                "name": "divide_number",
                "arguments": '{"number":17,"divisors":[2,3,4]}',
            },
        }

        result = agent.execute_tool_call(tool_call)

        self.assertEqual(result["role"], "tool")
        self.assertEqual(result["tool_call_id"], "call_123")
        self.assertEqual(result["name"], "divide_number")
        self.assertEqual(
            result["content"],
            (
                '{"results":[{"number":17,"divisor":2,"quotient":8,"remainder":1,'
                '"divides_evenly":false},{"number":17,"divisor":3,"quotient":5,'
                '"remainder":2,"divides_evenly":false},{"number":17,"divisor":4,'
                '"quotient":4,"remainder":1,"divides_evenly":false}]}'
            ),
        )


if __name__ == "__main__":
    unittest.main()
