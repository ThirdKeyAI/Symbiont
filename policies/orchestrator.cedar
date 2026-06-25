[
  {
    "name": "orchestrator_safe_tools",
    "active": true,
    "source": "permit(\n  principal,\n  action in [\n    Action::\"tool_call::list_agents\",\n    Action::\"tool_call::validate_dsl\",\n    Action::\"tool_call::validate_cedar\",\n    Action::\"tool_call::validate_toolclad\",\n    Action::\"tool_call::save_artifact\",\n    Action::\"tool_call::delegate\",\n    Action::\"tool_call::read_file\",\n    Action::\"tool_call::search\",\n    Action::\"tool_call::edit_file\"\n  ],\n  resource\n);"
  },
  {
    "name": "orchestrator_control_flow",
    "active": true,
    "source": "permit(\n  principal,\n  action in [Action::\"respond\", Action::\"terminate\"],\n  resource\n);"
  }
]
