# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Rust-based Raspberry Pi 3B garden irrigation controller. Controlled via Telegram bot over Tailscale VPN (LTE cellular, no WiFi at deployment site). See `docs/IRRIGATION_PLAN.md` for hardware inventory, wiring, and deployment details.

## Architecture

Two async loops running concurrently via `tokio::spawn`:

- **Telegram polling loop** (`src/telegram.rs`) — receives commands via teloxide `repl`, sends replies. Filters by `TELEGRAM_CHAT_ID`.
- **Scheduler loop** (`src/scheduler.rs`) — checks every 30s for schedule slot matches and auto-off timer expiry.

Shared state via `Arc<Mutex<AppState>>` and `Arc<Mutex<Valve>>`.

Key modules:
- `src/valve.rs` — GPIO 17 control via `rppal`. Compiles as stub on non-Linux (macOS dev).
- `src/state.rs` — `AppState` struct with JSON persistence. Schedule config, watering log, valve status.
- `src/main.rs` — entry point, signal handling, spawns both loops.

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
- `RUST_LOG` — optional, defaults to `irrigator=info`
