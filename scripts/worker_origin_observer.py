"""Correlate a live lease's reported origin with independently verified audit entries."""


def verify_origin(origin, entries, agent, run, public_key):
    assert origin and origin['agent_id'] == agent and origin['run_id'] == run
    assert origin['public_key'] == public_key
    matching = [entry for entry in entries if entry['event'].get('ToolDispatchStarted', {}).get('dispatch_id') == origin['dispatch_id']]
    assert len(matching) == 1, 'origin must identify exactly one signed dispatch'
    entry = matching[0]
    dispatch = entry['event']['ToolDispatchStarted']
    assert entry['agent_id'] == agent and entry['iteration'] == origin['iteration']
    assert dispatch['call_fingerprint'] == origin['call_fingerprint']
    assert dispatch['tool_name'] == origin['tool_name']
    assert any(prior['sequence'] < entry['sequence'] and any(
        call['fingerprint'] == origin['call_fingerprint']
        for call in prior['event'].get('PolicyEvaluated', {}).get('approved_calls', []))
        for prior in entries), 'origin dispatch must follow its exact policy checkpoint'
    return {'run_id': run, 'dispatch_id': origin['dispatch_id'], 'start_sequence': entry['sequence']}
