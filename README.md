# LangTrail

SDK-free agent trajectory capture and human-in-the-loop evaluation.

LangTrail is a Rust-based reverse proxy for AI agent traffic. It sits between agents and LLM providers, captures each request/response, and turns raw agent runs into structured trajectories that can be reviewed, scored, and exported as training or evaluation datasets.

## What It Does

- Captures OpenAI and Anthropic-compatible traffic without SDK changes or code instrumentation.
- Logs requests, responses, token usage, latency, estimated cost, model metadata, and tool calls.
- Groups events into agent trajectories using session and agent identifiers.
- Provides a React dashboard for inspecting events, reviewing trajectories, and saving human labels.
- Uses an LLM-assisted review layer to suggest critiques, labels, failure types, confidence scores, and quality signals.
- Keeps humans as the final decision-maker before labels are saved.
- Stores trajectories and reviews in PostgreSQL/TimescaleDB for querying and export.

## Core Workflow

```text
AI Agent
  |
  | OpenAI/Anthropic-compatible request
  v
LangTrail Proxy
  |
  | forwards request + captures response
  v
LLM Provider

Captured events -> PostgreSQL/TimescaleDB -> Review Dashboard -> Human labels -> Dataset export
```

## Quick Start

Requires Docker and Docker Compose.

```bash
docker compose -f docker/docker-compose.yml up -d
```

Services:

| Service | Port | Purpose |
| --- | --- | --- |
| Proxy | `4000` | Route agent LLM calls through this service |
| API | `4001` | REST API used by the dashboard |
| Dashboard | `3000` | Web UI for traces, costs, agents, and reviews |
| PostgreSQL/TimescaleDB | `5432` | Event and review storage |

Health checks:

```bash
curl http://localhost:4001/health
curl http://localhost:4001/ready
```

## Route Agent Traffic Through LangTrail

For OpenAI-compatible clients:

```bash
export OPENAI_BASE_URL=http://localhost:4000/proxy/openai/v1
export OPENAI_API_KEY=sk-...
```

For Anthropic-compatible clients:

```bash
export ANTHROPIC_BASE_URL=http://localhost:4000/proxy/anthropic/v1
export ANTHROPIC_API_KEY=sk-ant-...
```

Optional agent identifier:

```bash
x-agentland-agent-id: research-agent
```

## Example: Python OpenAI SDK

```python
from openai import OpenAI

client = OpenAI(
    api_key="sk-...",
    base_url="http://localhost:4000/proxy/openai/v1",
    default_headers={"x-agentland-agent-id": "research-agent"},
)

response = client.chat.completions.create(
    model="gpt-4",
    messages=[{"role": "user", "content": "Summarize this document"}],
)
```

## Human Review Flow

1. Agent calls flow through the proxy.
2. LangTrail captures request/response events and writes them asynchronously.
3. Events are grouped into trajectories by session.
4. Reviewers open a trajectory in the dashboard.
5. The LLM assistant suggests a label, critique, failure type, confidence score, and quality signals.
6. A human reviewer approves or edits the final label.
7. Reviewed trajectories can be exported for RLHF, preference data, and AI agent evaluation workflows.

## Review Labels

Supported trajectory labels:

- `good`
- `bad`
- `needs_review`

Supported failure types:

- `bad_answer`
- `bad_tool_use`
- `hallucination`
- `inefficient`
- `unsafe`
- `other`

## Tech Stack

Backend:

- Rust
- Tokio
- Hyper
- Axum
- Tower
- Reqwest
- Serde
- SQLx

Frontend:

- React
- TypeScript
- Vite
- Tailwind CSS
- React Query
- React Router
- Recharts

Data and infrastructure:

- PostgreSQL
- TimescaleDB
- Docker
- Docker Compose
- Kubernetes manifests
- Prometheus metrics

AI protocols and integrations:

- OpenAI-compatible APIs
- Anthropic-compatible APIs
- MCP
- A2A
- Server-Sent Events streaming

## API Surface

The dashboard uses REST APIs exposed by the Rust backend.

Key review endpoints:

```text
GET  /api/v1/reviews/trajectories
GET  /api/v1/reviews/trajectories/:session_id
POST /api/v1/reviews/trajectories/:session_id
POST /api/v1/reviews/trajectories/:session_id/assist
```

Other API areas:

- Events
- Agents
- Costs
- Projects
- Budgets
- Reports
- Health checks
- Metrics

## Project Structure

```text
.
├── crates/
│   ├── agentland-common/       # Shared types, config, protocol parsing
│   ├── agentland-store/        # PostgreSQL/TimescaleDB storage layer
│   ├── agentland-proxy/        # Reverse proxy, REST API, review handlers
│   ├── agentland-cli/          # Command-line interface
│   └── agentland-reports/      # Report generation utilities
├── dashboard/                  # React + TypeScript dashboard
├── docker/                     # Dockerfiles and Compose configs
├── init/                       # SQL migrations
├── k8s/                        # Kubernetes manifests
├── config/                     # Runtime config examples
└── scripts/                    # Setup, seed, test, and verification scripts
```

## Local Development

Start database and services:

```bash
docker compose -f docker/docker-compose.yml up -d
```

Run Rust tests:

```bash
cargo test --workspace
```

Run dashboard tests:

```bash
cd dashboard
pnpm install
pnpm test
```

Run the dashboard locally:

```bash
cd dashboard
pnpm dev
```

## Deployment Notes

LangTrail can run as Docker containers locally or in production-style infrastructure.

Included deployment assets:

- `docker/docker-compose.yml` for local multi-service deployment.
- `docker/Dockerfile` for the Rust proxy/API.
- `docker/Dockerfile.dashboard` for the React dashboard.
- `k8s/` manifests for Kubernetes deployment.

For a lightweight demo deployment, use:

- One containerized Rust backend/proxy.
- One PostgreSQL/TimescaleDB database.
- One static React dashboard.
