"""Exercise real init flags, prompts, validation and emitted review commands."""
import errno
import fcntl
import hashlib
import os
import pty
import select
import shlex
import subprocess
import termios
import time
import tomllib


def interactive_init(command, environment, answers):
    master, slave = pty.openpty()

    def terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                               env=environment, preexec_fn=terminal, close_fds=True)
    os.close(slave)
    output, pending = '', list(answers)
    deadline = time.monotonic() + 30
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if not data:
                    break
                output += data.decode(errors='replace')
                if pending and pending[0][0] in output:
                    _, value = pending.pop(0)
                    os.write(master, value.encode() + b'\n')
            if process.poll() is not None:
                break
        assert process.wait(timeout=5) == 0 and not pending, output
        return output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        os.close(master)


def check_initialization(binary, project, source, executable, provider_port):
    environment = dict(PATH='/usr/bin:/bin', HOME=os.environ['HOME'], LANG='C.UTF-8',
                       SYMBIONT_SANDBOX_STATE_DIR=str(project.parent/'state'))
    endpoint = f'http://127.0.0.1:{provider_port}'
    settings = dict(source=str(source), **{
        'managed-executable': str(executable), 'inference-url': endpoint,
        'inference-model': 'fixture', 'inference-key-env': 'FIXTURE_PROVIDER_KEY',
    })

    def arguments(destination, values=settings, profile='dev-agent'):
        return [str(binary), 'init', '--sandbox', 'landlock', '--profile', profile,
                '--schemapin', 'tofu', '--dir', str(destination),
                *[item for key, value in values.items() for item in ('--'+key, value)]]

    def run(command, success):
        result = subprocess.run(command, env=environment, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=30)
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        return result.stdout + result.stderr

    checks = []
    missing = project.parent/'missing-settings'
    result = run(arguments(missing, values={}), False)
    assert all('--'+name in result for name in settings), result
    assert not list(missing.iterdir()), 'missing settings created partial configuration'
    checks.append('noninteractive init names every missing setting before writing files')

    invalid = [
        ('overlap', {'source': str(project.parent)}),
        ('same-directory', {'source': str(project.parent/'invalid-same-directory')}),
        ('not-directory', {'source': str(source/'input.txt')}),
        ('colon', {'source': str(project.parent/'source:ambiguous')}),
        ('url-credential', {'inference-url': 'http://secret:password@localhost'}),
        ('url-query', {'inference-url': endpoint+'?secret=value'}),
        ('variable', {'inference-key-env': '9INVALID'}),
        ('runtime-variable', {'inference-key-env': 'SYMBIONT_MASTER_KEY'}),
        ('executable', {'managed-executable': str(source/'input.txt')}),
    ]
    (project.parent/'source:ambiguous').mkdir()
    alias = project.parent/'source-alias'
    alias.symlink_to(project.parent, target_is_directory=True)
    invalid.append(('alias-overlap', {'source': str(alias)}))
    for name, changes in invalid:
        destination = project.parent/('invalid-'+name)
        run(arguments(destination, values={**settings, **changes}), False)
        assert not list(destination.iterdir()), (name, 'partial configuration')
    run(arguments(project.parent/'wrong-profile', profile='assistant'), False)
    checks.append('overlapping roots, aliases, ambiguous paths and invalid provider/executable settings are refused')

    output = run(arguments(project), True)
    assert not (project/'docker-compose.yml').exists()
    assert sorted(path.stem for path in (project/'tools').iterdir()) == ['grep_files.clad', 'list_files.clad', 'read_file.clad']
    assert not (project/'policies/shell').exists()
    assert (project/'policies/managed-cli/dev-agent.cedar').is_file()
    assert (project/'DEVELOPMENT.md').is_file()
    assert 'FIXTURE_PROVIDER_KEY=' in (project/'.env.example').read_text()
    assert 'FIXTURE_PROVIDER_KEY=' not in (project/'.env').read_text()
    assert (project/'.env').stat().st_mode & 0o077 == 0
    run([str(binary), 'dsl', '--check', '-f', str(project/'agents/dev.symbi')], True)
    printed = next(line.strip() for line in output.splitlines() if line.strip().startswith('symbi run '))
    tokens = shlex.split(printed)
    assert tokens[:3] == ['symbi', 'run', 'dev']
    assert tokens[tokens.index('--target')+1] == str(source)
    checks.append('generated tools, scoped policies, configuration, private environment and review guidance agree')

    def snapshot():
        return {str(path.relative_to(project)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in project.rglob('*') if path.is_file()}
    before = snapshot()
    result = run(arguments(project), False)
    assert 'empty --dir' in result and 'Use --force' not in result
    run([*arguments(project), '--force'], False)
    assert snapshot() == before
    checks.append('reinitialization cannot overwrite or combine existing permissions, including with --force')

    prompted = project.parent/'prompted control'
    answers = [
        ('Source repository path', str(source)),
        ('Installed Claude Code executable path', str(executable)),
        ('Inference base URL', endpoint),
        ('Inference model', 'fixture'),
        ('Credential environment variable NAME', 'FIXTURE_PROVIDER_KEY'),
    ]
    interactive_init(arguments(prompted, values={}), environment, answers)
    assert tomllib.loads((prompted/'symbiont.toml').read_text()) == tomllib.loads((project/'symbiont.toml').read_text())
    checks.append('interactive prompts generate the same configuration as explicit flags')
    return dict(checks=checks, stdout=output, review_command=printed)
