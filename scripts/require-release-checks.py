#!/usr/bin/env python3
"""Require successful canonical GitHub workflows for one immutable commit.

Older successes never override a newer failure or an unfinished rerun. PR runs,
other repositories, and other commits cannot qualify a release. No credentials
are printed, and missing or unreadable evidence refuses publication.
"""
import argparse
import json
import os
import re
import time
import urllib.parse
import urllib.request

WORKFLOWS = {'test': 'test.yml', 'landlock': 'landlock-release.yml'}


def select_run(runs, repository, sha, workflow):
    eligible = [run for run in runs
                if run.get('head_sha') == sha
                and run.get('head_repository', {}).get('full_name', '').lower() == repository.lower()
                and run.get('path', '').split('@')[0] == '.github/workflows/' + workflow
                and run.get('event') in ('push', 'workflow_dispatch')
                and run.get('head_branch') == 'main']
    if not eligible:
        return None
    return max(eligible, key=lambda run: (run['id'], run.get('run_attempt', 1)))


def check(repository, sha, workflows, fetch):
    pending, evidence = [], []
    for workflow in workflows:
        query = urllib.parse.urlencode({'head_sha': sha, 'per_page': 100})
        data = fetch(f'/repos/{repository}/actions/workflows/{workflow}/runs?{query}')
        runs = data['workflow_runs']
        # Refuse ambiguity instead of accepting a possibly stale paginated run.
        if data['total_count'] > 100:
            raise RuntimeError(f'{workflow}: too many matching runs to qualify safely')
        run = select_run(runs, repository, sha, workflow)
        if run is None:
            pending.append(f'{workflow}: no canonical main run for {sha}')
        elif run['status'] != 'completed':
            pending.append(f'{workflow}: {run["status"]}')
        elif run.get('conclusion') != 'success':
            raise RuntimeError(f'{workflow}: latest run {run["id"]} concluded {run.get("conclusion")}')
        else:
            evidence.append({'workflow': workflow, 'run_id': run['id'],
                             'attempt': run.get('run_attempt', 1), 'sha': sha})
    return pending, evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sha', required=True)
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY'))
    parser.add_argument('--checks', choices=('test', 'all'), default='all')
    parser.add_argument('--wait-seconds', type=int, default=0)
    args = parser.parse_args()
    if not re.fullmatch(r'[0-9a-f]{40}', args.sha):
        parser.error('--sha must be a full commit SHA')
    if not args.repository or not re.fullmatch(r'[\w.-]+/[\w.-]+', args.repository):
        parser.error('--repository must be owner/name')
    if not 0 <= args.wait_seconds <= 7200:
        parser.error('--wait-seconds must be between 0 and 7200')
    token = os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN')
    if not token:
        parser.error('GH_TOKEN or GITHUB_TOKEN is required')

    def fetch(path):
        request = urllib.request.Request('https://api.github.com' + path, headers={
            'Authorization': 'Bearer ' + token,
            'Accept': 'application/vnd.github+json',
            'X-GitHub-Api-Version': '2022-11-28',
        })
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)

    workflows = [WORKFLOWS['test']] if args.checks == 'test' else list(WORKFLOWS.values())
    deadline = time.monotonic() + args.wait_seconds
    while True:
        pending, evidence = check(args.repository, args.sha, workflows, fetch)
        if not pending:
            print(json.dumps({'sha': args.sha, 'qualified': evidence}, indent=2))
            return
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError('; '.join(pending))
        print('; '.join(pending), flush=True)
        time.sleep(min(30, remaining))


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        raise SystemExit(f'Release qualification refused: {error}') from None
