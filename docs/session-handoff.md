# Agentland Session Handoff

## Repo

`/Users/anketpatil/Downloads/gautam/agentland`

## Product Direction

Agentland is being shaped as a Labelbox demo: collect agent trajectories through a proxy, make them readable, let humans label them, export Labelbox-style JSONL, and add lightweight AI reviewer assistance while preserving human oversight.

Focus the demo on trajectory collection, review, labeling, and dataset creation for RL/evals. Policy and PII can stay secondary for now unless they become necessary for the demo.

## User Preferences

- Keep the UI practical, tight, and demo-ready.
- Avoid bloated or generic features.
- Avoid large obvious buttons like `Generate AI Review`.
- Prefer small intuitive icon controls.
- Clicking the AI sparkle icon should request AI help and apply the suggestion immediately.
- Do not show a separate `Apply` button for AI suggestions.
- Clearly mark and highlight the matching failure step.
- Keep formatting compact; avoid excessive whitespace.
- Final responses should be concise and high-signal.

## Recent Feature State

- Added trajectory review flow in the dashboard.
- Added readable timeline and raw JSON toggle.
- Added Labelbox-style JSONL export.
- Added AI review assistant endpoint:
  - `POST /api/v1/reviews/trajectories/{session_id}/assist`
- AI review uses `AGENTLAND_REVIEW_ASSIST_OPENAI_API_KEY` or `OPENAI_API_KEY`.
- Default AI review model is `gpt-4o-mini`.
- AI response fields:
  - `suggested_label`
  - `confidence`
  - `failure_type`
  - `failure_step_index`
  - `failure_event_id`
  - `critique`
  - `quality_signals`
  - `model`
- Backend validates `failure_step_index` and maps it to a real event id.
- Dashboard sparkle icon now calls AI and auto-applies the returned label, failure type, notes, and failure step.
- Suggestion card shows `applied`; there is no Apply button.

## Prime Demo Behavior

The demo agent should receive one starting goal and available tools. The model must decide by itself whether to call the tool.

Current demo goal:

```text
Please determine whether 17 is prime.
```

Demo tool:

```text
divide_number
```

For the current captured run, the final answer is correct, but the tool call is unnecessary/inefficient. The correct AI review marking is:

```json
{
  "failure_step_index": 1,
  "failure_type": "bad_tool_use",
  "quality_signals": [
    "correct_final_answer",
    "unnecessary_tool_use",
    "tool_use_decision"
  ]
}
```

Step 1 should be highlighted because the model made the tool-use decision there. Do not mark the tool execution step unless the tool result itself is wrong.

## Key Files

- `dashboard/src/pages/ReviewPage.tsx`
  - Review UI, readable timeline, raw JSON, AI sparkle auto-apply.
- `dashboard/src/api/types.ts`
  - Review and AI assist types.
- `dashboard/src/api/client.ts`
  - Review API client.
- `dashboard/src/api/hooks.ts`
  - React Query hooks.
- `crates/agentland-proxy/src/api/handlers/reviews.rs`
  - Review endpoints, trajectory construction, AI assist, deterministic prime correction.
- `crates/agentland-proxy/src/api/router.rs`
  - Review routes.
- `crates/agentland-store/src/reviews.rs`
  - Review persistence.
- `scripts/test-multistep-agent.py`
  - Multi-step prime demo agent.
- `tests/test_multistep_agent_tools.py`
  - Tool/demo tests.

## Useful Commands

Build dashboard:

```bash
docker build -f docker/Dockerfile.dashboard -t agentland-dashboard-auto-apply-test .
```

Build proxy:

```bash
docker build -f docker/Dockerfile -t agentland-proxy-prime-json-test .
```

Restart full stack with AI key:

```bash
AGENTLAND_REVIEW_ASSIST_OPENAI_API_KEY=$(grep '^OPENAI_API_KEY=' .env.agentland-test | cut -d= -f2-) docker compose -f docker/docker-compose.yml up -d --build
```

Restart dashboard:

```bash
docker compose -f docker/docker-compose.yml up -d --build dashboard
```

Health check:

```bash
curl -s http://localhost:4001/health
```

Assist test:

```bash
curl -s -X POST http://localhost:4001/api/v1/reviews/trajectories/019e76e6-81f5-70d0-bf28-907b15de5c8b/assist
```

## Verification Already Done

- Dashboard Docker build passed after the auto-apply UI change.
- Dashboard/proxy compose restart reported:
  - `agentland-proxy Healthy`
  - `agentland-dashboard Started`
- `git diff --check` passed before final interruption.
- Final health check was interrupted by the user, so rerun it before claiming the stack is healthy.

## Caveats

- Docker and localhost `curl` often require escalated execution.
- Do not `source .env.agentland-test`; use `grep '^OPENAI_API_KEY=' .env.agentland-test`.
- The worktree contains many modified and untracked files. Do not revert user changes.
- `scripts/__pycache__` and `tests/__pycache__` are untracked generated files.

## Coding Guidelines Learned

- Use `rg` first for search.
- Use `apply_patch` for edits.
- Keep changes scoped.
- Do not over-explain in final answers.
- For code review requests, lead with bugs and risks.
- Verify before saying something is fixed.
- The user values fast progress and practical demo readiness.
