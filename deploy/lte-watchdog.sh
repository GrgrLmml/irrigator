#!/bin/bash
# Checks LTE connectivity via eth1. On failure, tries dhcpcd renewal, then reboot.
set -u
STATE_FILE=/var/lib/lte-watchdog-fails
FAIL_LIMIT=3

if ping -I eth1 -c 2 -W 3 1.1.1.1 > /dev/null 2>&1; then
    rm -f "$STATE_FILE"
    exit 0
fi

fails=$(cat "$STATE_FILE" 2>/dev/null || echo 0)
fails=$((fails + 1))
echo "$fails" > "$STATE_FILE"
logger -t lte-watchdog "ping via eth1 failed (count=$fails)"

if [ "$fails" -eq 2 ]; then
    logger -t lte-watchdog "attempting dhcpcd renewal on eth1"
    dhcpcd -k eth1 > /dev/null 2>&1
    sleep 2
    dhcpcd eth1 > /dev/null 2>&1
fi

if [ "$fails" -ge "$FAIL_LIMIT" ]; then
    logger -t lte-watchdog "reboot: $fails consecutive failures"
    rm -f "$STATE_FILE"
    /sbin/reboot
fi
