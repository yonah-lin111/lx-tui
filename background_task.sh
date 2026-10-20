#!/usr/bin/env bash
# Persistent background task: prints a heartbeat every 5s and never exits.
while true; do
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] bg-task alive: $(uptime)"
  sleep 5
done
