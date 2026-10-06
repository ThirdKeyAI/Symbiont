#!/usr/bin/env python3
"""Repeat a frozen intake candidate on held-out cases with two ordinary controls.

Uses real local Ollama inference and the installed CLI. All artifacts, including
runtime private keys, remain in a new private directory outside version control.
Exit 2 retains a completed experiment with quality failures; exit 1 is test error.
"""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import time
import urllib.parse
import urllib.request
import uuid


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def local_environment(endpoint, model):
    env = {k: v for k, v in os.environ.items() if not (
        k.endswith(('_API_KEY', '_API_KEY_REF', '_BASE_URL', '_PROXY'))
        or k.lower().endswith('_proxy')
        or k.startswith(('SYMBI', 'SYMBIONT', 'ANTHROPIC', 'OPENROUTER', 'OPENAI', 'CHAT_')))}
    env.update(OPENAI_API_KEY='local-ollama', OPENAI_BASE_URL=endpoint, CHAT_MODEL=model)
    return env


def classify(output, expected):
    if output == expected:
        return 'exact'
    labels = set(re.findall(r'\b(?:READY_FOR_REVIEW|REQUEST_DOCUMENTS|ESCALATE)\b', output))
    # Diagnostic only: the mandatory evaluator never normalizes the answer.
    return 'format_only' if labels == {expected} else 'routing_or_ambiguous'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--model', default='qwen3:8b')
    parser.add_argument('--endpoint', default='http://127.0.0.1:11434/v1')
    args = parser.parse_args()
    assert __debug__, 'Assertions must be enabled'
    endpoint = urllib.parse.urlsplit(args.endpoint)
    if (endpoint.scheme != 'http' or not endpoint.hostname
            or not ipaddress.ip_address(endpoint.hostname).is_loopback
            or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment
            or endpoint.path.rstrip('/') != '/v1'):
        parser.error('endpoint must be a literal loopback HTTP Ollama /v1 URL')
    binary = args.binary.resolve(strict=True)
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[1]
    previous = repo / 'examples/governed-improvements/document-intake'
    fixtures = repo / 'examples/governed-improvements/document-intake-phase2'
    for source, name in [(previous / 'intake.symbi', 'baseline.symbi'),
                         (previous / 'proposal-v1.json', 'proposal.json'),
                         (previous / 'contract.md', 'original-contract.md'),
                         (fixtures / 'suite.json', 'suite.json'),
                         (fixtures / 'contract.md', 'contract.md')]:
        shutil.copyfile(source, root / name)
    suite = json.loads((root / 'suite.json').read_text())
    old_suite = json.loads((previous / 'suite.json').read_text())
    assert not ({c['input'] for c in suite['cases']} & {c['input'] for c in old_suite['cases']})
    instructions = json.loads((root / 'proposal.json').read_text())['instructions']
    baseline = (root / 'baseline.symbi').read_text()
    # Same domain information as the candidate, carried in ordinary agent source.
    matched = baseline.rsplit('}', 1)[0] + ''.join(
        '    // ' + line + '\n' for line in instructions.splitlines()) + '}\n'
    projects = {}
    for name, source in [('general', baseline), ('matched', matched), ('candidate', baseline)]:
        project = root / name
        (project / 'agents').mkdir(mode=0o700, parents=True)
        (project / '.env').write_text('')
        (project / 'agents/intake.symbi').write_text(source)
        projects[name] = project
    env = local_environment(args.endpoint, args.model)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def model_identity():
        url = urllib.parse.urlunsplit((endpoint.scheme, endpoint.netloc, '/api/tags', '', ''))
        with opener.open(url, timeout=10) as response:
            models = json.load(response)['models']
        return next(m for m in models if m['name'] == args.model)

    report = dict(status='running', binary_sha256=digest(binary), driver_sha256=digest(__file__),
                  model=args.model, endpoint=args.endpoint, runs=[], evaluations=[], checks=[],
                  protocol=dict(repetitions=3, arms=['general', 'matched', 'candidate'],
                                promotion='none', exact_matches_required=12,
                                warmup='one unscored general-baseline run, retained separately'),
                  fixture_sha256={p.name: digest(p) for p in root.iterdir() if p.is_file()},
                  source_sha256={name: digest(p / 'agents/intake.symbi') for name, p in projects.items()},
                  limitations=[
                      'Engineering holdout, not independent domain-owner validation.',
                      'One model; repetitions are correlated, not independent statistical samples.',
                      'Matched control shares information but has different prompt packaging.',
                      'Synthetic metadata routing only; no record release or eligibility decisions.'])

    def checkpoint():
        (root / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')

    def cli(project, *words, expected=(0,)):
        started = time.monotonic()
        result = subprocess.run([str(binary), *map(str, words)], cwd=project, env=env,
                                capture_output=True, text=True, timeout=180)
        seconds = time.monotonic() - started
        with (root / 'commands.jsonl').open('a') as log:
            log.write(json.dumps(dict(project=str(project), arguments=list(map(str, words)),
                                      code=result.returncode, seconds=round(seconds, 3),
                                      stdout=result.stdout, stderr=result.stderr)) + '\n')
        if result.returncode not in expected:
            raise RuntimeError(f'{words[:3]} failed ({result.returncode}): {result.stderr}')
        return result, seconds

    def admin(operation, *words, expected=(0,)):
        result, _ = cli(projects['candidate'], 'improvement', operation, '--workflow', 'intake',
                        *words, expected=expected)
        return json.loads(result.stdout)

    def execute(arm, repeat, case, candidate=None):
        project = projects[arm]
        invocation = str(uuid.uuid4())
        words = ['run', 'intake', '--input', case['input'], '--max-iterations', '2',
                 '--invocation-id', invocation]
        if candidate:
            words += ['--improvement', 'intake', '--improvement-trial', candidate]
        result, seconds = cli(project, *words)
        fields = dict(re.findall(r'^Audit (run|journal|public key): (.+)$', result.stderr, re.MULTILINE))
        audit = dict(run_id=fields['run'], path=fields['journal'], public_key=fields['public key'])
        verified, _ = cli(project, 'audit', 'inspect', audit['path'], '--run-id', audit['run_id'],
                          '--public-key', audit['public_key'])
        (root / f'audit-{invocation}.json').write_text(verified.stdout)
        cached, _ = cli(project, *words)
        output = cached.stdout.removesuffix('\n')
        events = [json.loads(line)['payload']['entry']['event']
                  for line in Path(audit['path']).read_text().splitlines()]
        binding = events[0]['Started']['execution_context'].get('improvement')
        assert not any('ToolDispatchStarted' in event for event in events), 'unexpected dispatch'
        assert events[-1]['Terminated']['reason'] == 'Completed'
        if candidate:
            assert events[-2]['ImprovementOutput']['output'] == output
            assert binding['mode'] == 'trial' and binding['candidate'] == candidate
        else:
            assert binding is None
        row = dict(arm=arm, repeat=repeat, case=case['id'], expected=case['expected_output'],
                   actual=output, classification=classify(output, case['expected_output']),
                   invocation_id=invocation, audit=audit, binding=binding,
                   tokens=events[-1]['Terminated']['total_usage']['total_tokens'],
                   seconds=round(seconds, 3))
        if repeat:
            report['runs'].append(row)
        else:
            report['warmup'] = row
        checkpoint()
        print(f"{repeat}/{arm}/{case['id']}: {row['classification']} {output!r}", flush=True)
        return row

    initialized = False
    exit_code = 1
    try:
        report['model_identity'] = model_identity()
        checkpoint()
        for project in projects.values():
            cli(project, 'dsl', '--check', '-f', 'agents/intake.symbi')
        execute('general', 0, old_suite['cases'][0])
        state = admin('init', '--agent', 'agents/intake.symbi', '--suite', root / 'suite.json')
        initialized = True
        report['workflow_public_key'] = state['public_key']
        candidate = admin('propose', '--file', root / 'proposal.json')['candidate']
        report['candidate'] = candidate
        checkpoint()
        arms = ['general', 'matched', 'candidate']
        for repeat in range(1, 4):
            refs = []
            for index, case in enumerate(suite['cases']):
                offset = (repeat - 1 + index) % len(arms)
                for arm in arms[offset:] + arms[:offset]:
                    row = execute(arm, repeat, case, candidate if arm == 'candidate' else None)
                    if arm == 'candidate':
                        refs.append(dict(case_id=case['id'], audit=row['audit']))
            path = root / f'trials-{repeat}.json'
            path.write_text(json.dumps(refs, indent=2) + '\n')
            evaluation = admin('evaluate', '--candidate', candidate, '--trials', path, expected=(0, 2))
            report['evaluations'].append(evaluation)
            if not evaluation['report']['accepted']:
                cli(projects['candidate'], 'improvement', 'approve', '--workflow', 'intake',
                    '--candidate', candidate, '--evaluation', evaluation['evaluation'],
                    '--rationale', 'Negative acceptance check: a failed evaluation must refuse approval',
                    expected=(1,))
                report['checks'].append(f'repetition {repeat}: rejected evaluation cannot be approved')
            checkpoint()
        for arm in ['general', 'matched']:
            assert not (projects[arm] / '.symbiont/improvements').exists()
        report['checks'].append('both ordinary controls remain outside the improvement lifecycle')
        assert model_identity()['digest'] == report['model_identity']['digest']
        assert {p.name: digest(p) for p in root.iterdir()
                if p.name in report['fixture_sha256']} == report['fixture_sha256']
        assert {name: digest(p / 'agents/intake.symbi') for name, p in projects.items()} == report['source_sha256']
        accepted = all(e['report']['accepted'] for e in report['evaluations'])
        report['status'] = 'completed' if accepted else 'quality_refused'
        exit_code = 0 if accepted else 2
    except Exception as error:
        report.update(status='test_error', error=repr(error))
    finally:
        if initialized:
            try:
                admin('disable')
                report['final_workflow_state'] = admin('inspect')['workflow']
            except Exception as error:
                report['cleanup_error'] = repr(error)
                exit_code = 1
        report['summary'] = {}
        for arm in ['general', 'matched', 'candidate']:
            rows = [r for r in report['runs'] if r['arm'] == arm]
            if rows:
                report['summary'][arm] = dict(
                    cases=len(rows), exact=sum(r['classification'] == 'exact' for r in rows),
                    format_only=sum(r['classification'] == 'format_only' for r in rows),
                    routing_or_ambiguous=sum(r['classification'] == 'routing_or_ambiguous' for r in rows),
                    tokens=sum(r['tokens'] for r in rows),
                    median_seconds=round(statistics.median(r['seconds'] for r in rows), 3),
                    p95_seconds=sorted(r['seconds'] for r in rows)[max(0, (95 * len(rows) + 99) // 100 - 1)])
        checkpoint()
        print(json.dumps(dict(status=report['status'], summary=report['summary'],
                              report=str(root / 'report.json'))), flush=True)
    return exit_code


if __name__ == '__main__':
    raise SystemExit(main())
