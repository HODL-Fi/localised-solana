#!/bin/bash
# Faithful to the upstream /app/entrypoint.sh, with one change: the listening port comes from the
# environment. Railway injects $PORT and expects the process to bind it.
start_bun_app() {
  INDEX=/app/dist/index.js
  if [ ! -f "$INDEX" ]; then
    INDEX=dist/index.js
  fi
  PORT="${PORT:-8080}" bun run "$INDEX" &
  BUN_APP_PID=$!
}

start_bun_app
echo "crossbar listening on ${PORT:-8080}"

# Upstream restarts the app if it dies rather than exiting, so a transient crash does not take the
# service down. Kept, because Railway would restart the container anyway but slower.
monitor_processes() {
    while true; do
        if ! kill -0 $BUN_APP_PID >/dev/null 2>&1; then
            echo "Bun app died, restarting..."
            start_bun_app
        fi
        sleep 5
    done
}

monitor_processes
