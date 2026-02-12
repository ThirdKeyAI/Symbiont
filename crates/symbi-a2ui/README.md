# symbi-a2ui

Operations console for the Symbiont agent runtime. Renders three panels — Fleet Overview, Audit Trail, and Compliance Dashboard — on top of the runtime REST API.

## Tech Stack

- **Lit 3** web components
- **Vite 6** build toolchain
- **Tailwind CSS v4**
- **TypeScript**
- **Caddy 2** reverse proxy + static file server

## Quick Start

```bash
npm install
npm run dev        # http://localhost:5173  (proxies /api → localhost:8080)
```

On first load, you'll be prompted to enter your `SYMBI_AUTH_TOKEN`.

## Build

```bash
npm run build      # outputs to dist/
```

## Docker

```bash
docker build -t symbi-a2ui .
docker run -p 3001:3001 symbi-a2ui
```

The container serves the console on port 3001 and reverse proxies `/api/*` to `symbi:8080` on the Docker network.

## Panels

| Panel | Polling | API Endpoints |
|-------|---------|---------------|
| Fleet Overview | 10s | agents, schedules, health, scheduler health, metrics |
| Audit Trail | 15s | agent history, schedule history, channel audit |
| Compliance | 30s | agents status, scheduler health, channel health (scores computed client-side) |

## Architecture

All runtime data flows through the existing REST API on port 8080. The UI is a thin rendering layer — no server-side state, no database. Auth token is stored in `localStorage`.

Created: 2026-02-11
