#!/usr/bin/env sh
# Evaluate two tool calls against the policy in this directory.
# Offline: no API key, no model, no network.
set -eu
cd "$(dirname "$0")"

echo '{"tool_name":"list_agents"}'   | symbi policy evaluate --stdin --policies . --json
echo '{"tool_name":"system_health"}' | symbi policy evaluate --stdin --policies . --json
