// A privileged tool the agent must never call, and a safe one it may.
// The runtime evaluates these before dispatching any tool call.
forbid(principal, action == Symbi::Action::"tool_call::list_agents",   resource);
permit(principal, action == Symbi::Action::"tool_call::system_health", resource);
