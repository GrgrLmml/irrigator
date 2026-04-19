# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Rust-based Raspberry Pi 3B garden irrigation controller. Controlled via Telegram bot over Tailscale VPN (LTE cellular, no WiFi at deployment site). See `docs/IRRIGATION_PLAN.md` for hardware inventory, wiring, and deployment details.

## Architecture

Three async loops running concurrently via `tokio::spawn`:

- **Telegram polling loop** (`src/telegram.rs`) — receives commands via teloxide `repl`, sends replies. Filters by `TELEGRAM_CHAT_ID`.
- **Scheduler loop** (`src/scheduler.rs`) — checks every 30s for schedule slot matches, auto-off timer, and periodic flow reports.
- **Web server** (`src/web.rs`) — axum HTTP server on `IRRIGATOR_BIND_ADDR` (default `0.0.0.0:8080`). Serves the mobile UI (`ui/` embedded via `include_str!`) and the JSON API.

Shared state: `Arc<Mutex<AppState>>`, `Arc<Mutex<Valve>>`, `Arc<Mutex<FlowSensor>>`.

Key modules:
- `src/valve.rs` — GPIO 17 control via `rppal`. Stub on non-Linux.
- `src/flow.rs` — GPIO 22 pulse-counting Hall-effect sensor via `rppal::Gpio::set_async_interrupt`. Stub on non-Linux.
- `src/state.rs` — `AppState` struct with JSON persistence. Schedule config, watering log, lifetime totals, `start_session`/`finish_session` helpers.
- `src/web.rs` — HTTP handlers, JSON DTOs, embedded static assets. Web mutations echo to Telegram via `telegram::notify`.
- `src/main.rs` — entry point, signal handling, spawns all three loops.

### Lock Ordering

`state → flow → valve`. All handlers must snapshot fields into local DTOs and drop the mutex guard BEFORE serializing JSON or awaiting anything other than another mutex.

### UI Dev Loop

Set `IRRIGATOR_UI_DIR=./ui` to serve static assets from disk (no rebuild needed). Release builds use the embedded copies. The standalone design mockup is at `ui/mockup.html` — open directly in a browser for offline design iteration.

## Key Constraints

- **Target**: Raspberry Pi 3B, Raspberry Pi OS Lite 64-bit, ARMv7
- **GPIO**: pin 17 (BCM), active-high relay. `rppal` crate, only compiles on Linux. Stubbed on other platforms via `#[cfg(target_os)]`.
- **Safety invariants**: valve always OFF on boot/restart/shutdown; every open has max duration (default 120 min); SIGTERM closes valve.
- **Reserved GPIO pins**: 2, 3 (I2C), 27 (zone 2), 22 (zone 3), 4 (temp sensor)

## Build & Deploy

```bash
# Local dev (stub GPIO)
cargo check
cargo run  # needs TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID in .env

# Cross-compile for Pi
cross build --release --target armv7-unknown-linux-gnueabihf

# Deploy
scp target/armv7-unknown-linux-gnueabihf/release/irrigator irrigator@<tailscale-ip>:/usr/local/bin/
sudo systemctl restart irrigator
journalctl -u irrigator -f
```

## Environment Variables

- `TELEGRAM_BOT_TOKEN` — from @BotFather
- `TELEGRAM_CHAT_ID` — numeric chat ID, only this chat can send commands
- `IRRIGATOR_BIND_ADDR` — optional, defaults to `0.0.0.0:8080`
- `IRRIGATOR_WEB_TOKEN` — optional, if set the web UI requires `?t=<token>` or `X-Irrigator-Token` header (not yet enforced in handlers; groundwork only)
- `IRRIGATOR_UI_DIR` — optional dev override, serves `ui/` from disk instead of embedded copies
- `RUST_LOG` — optional, defaults to `irrigator=info`
