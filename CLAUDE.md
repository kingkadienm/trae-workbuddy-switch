# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Buddy Switch — a multi-account management tool for WorkBuddy / TraeWork / Trae clients (OAuth QR login, one-click account switching, credits/checkin monitoring, token usage stats), which can also expose model quota as OpenAI / Anthropic compatible local API endpoints. One frontend, two deployment forms:

- **Desktop App**: Tauri (`src-tauri/`), frontend talks to Rust via Tauri commands
- **webui**: `crates/buddy-switch-server/` serves the same frontend over local HTTP (`127.0.0.1:57890`); `dist/` is embedded into the server binary at compile time via `rust-embed`

Project and user-facing docs are in Chinese; commits use Conventional Commit prefixes with Chinese subjects/bodies.

## Commands

```bash
# Frontend dev (Vite on :1420, proxies /api/* and /v1/* to :57890)
npm run dev

# Rust backend for webui dev
cd crates/buddy-switch-server && cargo run -- --port 57890

# Desktop app dev
npm run tauri dev

# Frontend build (runs tsc + all contract checks below, then vite build)
npm run build

# Contract checks — MUST pass after any API change (enforced in build & CI)
npm run check:api               # frontend/backend API command-name + route parity
npm run check:store-selectors   # zustand store selector usage
npm run check:npm-platforms     # npm platform package consistency

# Rust
cargo build --workspace
cargo test --workspace
cargo test -p buddy-switch-core                # single crate
cargo test -p buddy-switch-core token_stats    # single test by name filter

# Demo mode (fake data, for screenshots / GitHub Pages)
npm run dev:demo
```

Build order matters for webui: frontend (`npm run build`) **before** compiling the server, because `dist/` is embedded at compile time. Stop a running `buddy-switch` binary before recompiling on Windows (file lock).

Do not change the `[profile.dev]`/`[profile.test]` debug settings in the root `Cargo.toml` — they exist to keep `target/` size down (see the extensive comment there); `release` profile is deliberately untouched because release caching speeds up packaging.

## Architecture

Cargo workspace members + frontend:

| Crate | Role |
|---|---|
| `crates/buddy-switch-core` | All business logic as modules (`src/modules/`): accounts, OAuth, switch, credits, checkin, token stats, scheduler, etc. Product-specific code lives in submodules: `modules/trae/` (Trae/TraeWork), `modules/doubao/` (in development), `modules/growth/`. |
| `crates/buddy-switch-gateway` | OpenAI/Anthropic-compatible relay gateway. `protocol/` (per-protocol request/response translation), `routes/` (chat/messages/models/relay endpoints), `pool/` (account pools), `sticky/` (sticky sessions), `account_strategy.rs`, `model_route.rs`, `outbound/` (upstream calls), `trae/` (Trae-specific gateway logic). |
| `crates/buddy-switch-server` | webui binary. `api.rs` (huge — REST route table mirroring Tauri commands) + embedded frontend assets. `/api/*` = management API, `/v1/*` = gateway. |
| `src-tauri/` | Desktop app. `commands.rs` (~2300 lines of Tauri commands) wraps buddy-switch-core; `lib.rs` registers commands in `generate_handler!`; `gateway.rs`/`trae_gateway.rs` host the gateway in-process; `tray.rs` for the tray. |
| `src/` | React 19 + TypeScript + Tailwind 4 + shadcn/Radix + zustand frontend. `lib/api.ts` is the single API layer (see below). `stores/` (zustand), `pages/` (per-page components), `lib/trae-*.ts` (Trae side client). |

### The dual-channel API layer (most important invariant)

Every frontend call goes through `call("<cmd>")` in [src/lib/api.ts](src/lib/api.ts). The same command name must exist in **three places**:

1. `ROUTES` table in `src/lib/api.ts` — maps cmd → `{ method, path }`
2. `src-tauri/src/lib.rs` `generate_handler!` — desktop command registration
3. `crates/buddy-switch-server/src/api.rs` route table — webui HTTP route

There is no compile-time cross-check between TS and Rust; drift = "builds green, silently broken at runtime". `npm run check:api` (see [docs/API_CONTRACT.md](docs/API_CONTRACT.md)) validates all 7 parity rules, including that read-only demo commands have `screenshotDemoResponse` cases and that single-`Value`-arg commands match Rust parameter names. **Run it after any API change.**

### Two product partitions

The UI has independent WorkBuddy and TraeWork partitions (sidebar switch), each with its own account store, clients, and config; page structures are isomorphic (Accounts / Token Stats / Credit Stats / API Service / Settings). Trae has additional internal variants (Trae Work vs Trae IDE) carried by `?line=` in the URL. Trae-side code is consistently prefixed `trae_` (commands, files, components).

### Demo mode

`VITE_DEMO_MODE=1` makes the frontend return fake data for read-only commands (GitHub Pages demo). `DEMO_READ_COMMANDS` and `screenshotDemoResponse` in `src/lib/` must stay in sync with the real API surface — `check:api` enforces this.

## UI conventions

- Use existing shadcn/Radix components under `src/components/ui/` and compose them before writing custom interactive primitives; add missing shadcn components there rather than rolling custom ones.
- Chinese font stack, CSS-variable-based design tokens, no emoji as icons (use lucide-react SVG icons).

## Trellis

This project is managed by Trellis (see `AGENTS.md`): workflow knowledge lives under `.trellis/` (workflow phases, per-layer coding specs in `.trellis/spec/`, task journals in `.trellis/workspace/`). Read the relevant spec before writing code in a given layer, and prefer Trellis commands (`/trellis:finish-work`, `/trellis:continue`) over manual steps when available.
