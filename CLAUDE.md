# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Raspberry Pi 3B-based garden irrigation controller. Remotely controlled via REST API over Tailscale VPN (LTE cellular, no WiFi at site), with SMS fallback via ZTE MF833U1 dongle HTTP API. See `docs/IRRIGATION_PLAN.md` for full hardware inventory, wiring, and deployment steps.

## Architecture

The system has these core components:
- **REST API** (Axum or FastAPI) — valve control, schedule management, status/log endpoints on port 8080
- **Schedule Runner** — async loop that fires watering slots (default: 4×/day, 8 min each for germination)
- **Command Processor** — shared text command interface used by both REST API and SMS handler
- **Valve Controller** — GPIO 17 control (active-high relay → 12V NC solenoid valve)
- **State Manager** — JSON file persistence at `/etc/irrigator/state.json`, config at `/etc/irrigator/config.json`
- **SMS Handler** — polls ZTE dongle HTTP API for incoming SMS, sends responses
- **Sensor Receiver** (future) — ESP32 serial USB listener for soil moisture data

All inputs (API, SMS, schedule) flow through the Command Processor to the Valve Controller. State is persisted to JSON on every change.

## Key Constraints

- **Target**: Raspberry Pi 3B, Raspberry Pi OS Lite 64-bit (Debian Bookworm), ARMv7
- **Cross-compilation** (Rust): target `armv7-unknown-linux-gnueabihf`, use `cross` tool
- **GPIO**: pin 17 (BCM), active-high. Relay VCC on 3.3V. Pi GND must share common ground with 12V PSU
- **Safety invariants**: valve always OFF on boot/restart; every open command has max duration (default 120 min); SIGTERM/SIGINT must close valve before exit
- **Reserved GPIO pins**: 2, 3 (I²C for future ADC), 27 (zone 2), 22 (zone 3), 4 (temp sensor)
- **SMS commands accepted only from configured owner phone number**

## Build & Deploy (Rust path)

```bash
# Cross-compile for Pi
cross build --release --target armv7-unknown-linux-gnueabihf

# Deploy binary
scp target/armv7-unknown-linux-gnueabihf/release/irrigator pi@irrigator:/usr/local/bin/

# Service management on Pi
sudo systemctl restart irrigator
sudo systemctl status irrigator
journalctl -u irrigator -f
```

## Build & Deploy (Python path)

```bash
# On Pi
/opt/irrigator/venv/bin/pip install -r requirements.txt
sudo systemctl restart irrigator
```
