#!/usr/bin/env python3
"""Run a fixed document-intake pilot with real local Ollama inference.

Retains synthetic inputs, CLI logs, signed journals and a comparison report in a
new private output directory. No cloud credentials, tool grants or customer data.
Quality failures are retained and never waived to complete promotion.
"""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import urllib.parse
import urllib.request
import uuid


class AcceptanceFailure(Exception):
    """A completed evaluation rejected the fixed acceptance criteria."""


def digest(path):
    hasher = hashlib.sha256()
    with path.open('rb') as source:
        while chunk := source.read(1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--model', default='qwen3:8b')
    parser.add_argument('--endpoint', default='http://127.0.0.1:11434/v1')
    args = parser.parse_args()
    endpoint = urllib.parse.urlsplit(args.endpoint)
    if (endpoint.scheme != 'http' or not endpoint.hostname
            or not ipaddress.ip_address(endpoint.hostname).is_loopback
            or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment
            or endpoint.path.rstrip('/') != '/v1'):
        parser.error('endpoint must be a literal loopback HTTP Ollama /v1 URL')
    binary = args.binary.resolve(strict=True)
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    fixtures = Path(__file__).resolve().parents[1] / 'examples/governed-improvements/document-intake'
    project = root / 'project'
    (project / 'agents').mkdir(mode=0o700, parents=True)
    (project / '.env').write_text('')
    shutil.copyfile(fixtures / 'intake.symbi', project / 'agents/intake.symbi')
    for name in ['suite.json', 'proposal-v1.json', 'proposal-v2.json', 'contract.md']:
        shutil.copyfile(fixtures / name, root / name)
    suite = json.loads((root / 'suite.json').read_text())
    env = {k: v for k, v in os.environ.items() if not (
        k.endswith(('_API_KEY', '_API_KEY_REF', '_BASE_URL', '_PROXY'))
        or k.lower().endswith('_proxy')
        or k.startswith(('SYMBI', 'SYMBIONT', 'ANTHROPIC', 'OPENROUTER', 'OPENAI', 'CHAT_')))}
    env.update(OPENAI_API_KEY='local-ollama', OPENAI_BASE_URL=args.endpoint, CHAT_MODEL=args.model)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def model_identity():
        url = urllib.parse.urlunsplit((endpoint.scheme, endpoint.netloc, '/api/tags', '', ''))
        with opener.open(url, timeout=10) as response:
            models = json.load(response)['models']
        return next(m for m in models if m['name'] == args.model)

    report = dict(status='running', model=args.model, endpoint=args.endpoint,
                  binary_sha256=digest(binary), fixture_sha256={
                      p.name: digest(p) for p in sorted(fixtures.iterdir()) if p.is_file()},
                  runs=[], evaluations={}, lifecycle=[], limitations=[
                      'Synthetic engineering cases; no domain-owner signoff or regulatory validation.',
                      'One local model and one run per case/version; no statistical generalization claim.',
                      'Routing text only. No tools registered and no record release or access changes.',
                      'Both candidate prompts and all expected outputs fixed before inference.'])

    def checkpoint():
        (root / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')

    def cli(*words, expected=(0,)):
        started = time.monotonic()
        result = subprocess.run([str(binary), *map(str, words)], cwd=project, env=env,
                                capture_output=True, text=True, timeout=180)
        with (root / 'commands.jsonl').open('a') as log:
            log.write(json.dumps(dict(arguments=list(map(str, words)), code=result.returncode,
                                      seconds=round(time.monotonic() - started, 3),
                                      stdout=result.stdout, stderr=result.stderr)) + '\n')
        if result.returncode not in expected:
            raise RuntimeError(f'{words[:3]} failed ({result.returncode}): {result.stderr}')
        return result

    def admin(operation, *words, expected=(0,)):
        return json.loads(cli('improvement', operation, '--workflow', 'intake', *words,
                              expected=expected).stdout)

    def check(name):
        report['lifecycle'].append(name)
        checkpoint()
        print('PASS', name, flush=True)

    def execute_case(phase, case, candidate=None, selected=False):
        invocation = str(uuid.uuid4())
        words = ['run', 'intake', '--input', case['input'], '--max-iterations', '2',
                 '--invocation-id', invocation]
        if candidate or selected:
            words += ['--improvement', 'intake']
        if candidate:
            words += ['--improvement-trial', candidate]
        started = time.monotonic()
        result = cli(*words)
        fields = dict(re.findall(r'^Audit (run|journal|public key): (.+)$', result.stderr, re.MULTILINE))
        audit = dict(run_id=fields['run'], path=fields['journal'], public_key=fields['public key'])
        verified = json.loads(cli('audit', 'inspect', audit['path'], '--run-id', audit['run_id'],
                                  '--public-key', audit['public_key']).stdout)
        # Same invocation returns its retained final receipt without inference.
        cached = cli(*words).stdout
        output = cached[:-1] if cached.endswith('\n') else cached
        entries = [json.loads(line)['payload']['entry'] for line in Path(audit['path']).read_text().splitlines()]
        events = [entry['event'] for entry in entries]
        binding = events[0]['Started']['execution_context'].get('improvement')
        assert not any('ToolDispatchStarted' in event for event in events), 'unexpected side effect'
        assert events[-1]['Terminated']['reason'] == 'Completed'
        if candidate or selected:
            assert events[-2]['ImprovementOutput']['output'] == output
            assert binding['mode'] == ('trial' if candidate else 'approved')
        else:
            assert binding is None, 'ordinary run unexpectedly opted in'
        row = dict(phase=phase, case=case['id'], expected=case['expected_output'], actual=output,
                   passed=output == case['expected_output'], invocation_id=invocation,
                   audit=audit, binding=binding, tokens=events[-1]['Terminated']['total_usage']['total_tokens'],
                   seconds=round(time.monotonic() - started, 3))
        report['runs'].append(row)
        (root / f'audit-{invocation}.json').write_text(json.dumps(verified, indent=2) + '\n')
        checkpoint()
        print(f"{phase}: {case['id']} => {output!r} ({'PASS' if row['passed'] else 'FAIL'})", flush=True)
        return row

    def trial(version):
        candidate = admin('propose', '--file', root / f'proposal-v{version}.json')['candidate']
        references = []
        for case in suite['cases']:
            row = execute_case(f'candidate_v{version}', case, candidate=candidate)
            references.append(dict(case_id=case['id'], audit=row['audit']))
        path = root / f'trials-v{version}.json'
        path.write_text(json.dumps(references, indent=2) + '\n')
        evaluation = admin('evaluate', '--candidate', candidate, '--trials', path, expected=(0, 2))
        report['evaluations'][f'v{version}'] = evaluation
        checkpoint()
        if not evaluation['report']['accepted']:
            cli('improvement', 'approve', '--workflow', 'intake', '--candidate', candidate,
                '--evaluation', evaluation['evaluation'], '--rationale', 'Pilot failed; approval must refuse',
                expected=(1,))
            check(f'failed v{version} candidate refused approval')
            raise AcceptanceFailure(f'Candidate v{version} failed the frozen acceptance criteria; not promoted')
        approval = admin('approve', '--candidate', candidate, '--evaluation', evaluation['evaluation'],
                         '--rationale', 'Synthetic local pilot: reviewed all fixed routing cases and signed evidence')['approval']
        return candidate, approval, evaluation['evaluation']

    initialized = False
    exit_code = 1
    try:
        report['model_identity'] = model_identity()
        checkpoint()
        cli('dsl', '--check', '-f', 'agents/intake.symbi')
        for case in suite['cases']:
            execute_case('baseline', case)
        assert not (project / '.symbiont/improvements').exists()
        check('ordinary baseline creates no improvement workflow')
        state = admin('init', '--agent', 'agents/intake.symbi', '--suite', root / 'suite.json')
        initialized = True
        report['workflow_public_key'] = state['public_key']
        checkpoint()
        cli('run', 'intake', '--improvement', 'intake', expected=(1,))
        check('initialization alone does not activate a candidate')
        first, approval, evaluation = trial(1)
        admin('promote', '--candidate', first, '--approval', approval, '--expected-active', 'none')
        row = execute_case('active_v1', suite['cases'][0], selected=True)
        assert row['passed'] and row['binding']['candidate'] == first and row['binding']['approval'] == approval
        check('approved first version runs with exact signed attribution')
        second, second_approval, _ = trial(2)
        cli('improvement', 'promote', '--workflow', 'intake', '--candidate', second,
            '--approval', second_approval, '--expected-active', 'none', expected=(1,))
        check('stale activation refuses without replacing current version')
        admin('promote', '--candidate', second, '--approval', second_approval, '--expected-active', first)
        row = execute_case('active_v2', suite['cases'][1], selected=True)
        assert row['passed'] and row['binding']['candidate'] == second
        admin('rollback', '--candidate', first, '--approval', approval, '--expected-active', second)
        row = execute_case('rolled_back_v1', suite['cases'][1], selected=True)
        assert row['passed'] and row['binding']['candidate'] == first
        check('rollback restores the previously approved version for new runs')
        for kind, identifier in [('candidate', first), ('evaluation', evaluation), ('approval', approval)]:
            document = admin('export', '--kind', kind, '--id', identifier)
            path = root / f'{kind}.signed.json'
            path.write_text(json.dumps(document, indent=2) + '\n')
            cli('improvement', 'verify', '--kind', kind, '--file', path, '--public-key', state['public_key'])
        check('exported candidate, evaluation and approval verify independently')
        admin('disable')
        before = set((project / '.symbiont/governed').glob('*.jsonl'))
        refused = cli('run', 'intake', '--improvement', 'intake', expected=(1,))
        assert 'disabled' in refused.stderr
        assert before == set((project / '.symbiont/governed').glob('*.jsonl'))
        execute_case('disabled_ordinary', suite['cases'][0])
        check('disabled selection refuses before execution; ordinary execution remains independent')
        assert model_identity()['digest'] == report['model_identity']['digest']
        report['status'] = 'completed'
        exit_code = 0
    except AcceptanceFailure as error:
        report.update(status='quality_refused', error=str(error))
        exit_code = 2
    except Exception as error:
        report.update(status='test_error', error=str(error))
    finally:
        if initialized:
            try:
                admin('disable')
                report['final_workflow_state'] = admin('inspect')['workflow']
            except Exception as error:
                report['cleanup_error'] = str(error)
                exit_code = 1
        report['summary'] = {}
        for phase in sorted({row['phase'] for row in report['runs']}):
            rows = [row for row in report['runs'] if row['phase'] == phase]
            report['summary'][phase] = dict(passed=sum(row['passed'] for row in rows), cases=len(rows),
                                           tokens=sum(row['tokens'] for row in rows))
        checkpoint()
        print(json.dumps(dict(status=report['status'], summary=report['summary'],
                              report=str(root / 'report.json'))), flush=True)
    return exit_code


if __name__ == '__main__':
    raise SystemExit(main())
