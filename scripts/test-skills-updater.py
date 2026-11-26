#!/usr/bin/env python3
"""Exercise the shipping signed-rule updater against a temporary local HTTPS feed."""
import argparse
import functools
import http.server
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    binary = parser.parse_args().binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='symbi-rule-feed-') as temporary:
        root = Path(temporary)
        private, public = root / 'publisher.private.pem', root / 'publisher.pem'
        cert, tls_key = root / 'server.pem', root / 'server.key'

        def openssl(*args):
            subprocess.run(['openssl', *map(str, args)], check=True, capture_output=True)

        openssl('genpkey', '-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256', '-out', private)
        openssl('pkey', '-in', private, '-pubout', '-out', public)
        openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', tls_key,
                '-out', cert, '-days', '1', '-subj', '/CN=localhost',
                '-addext', 'subjectAltName=DNS:localhost')
        env = dict(os.environ, SSL_CERT_FILE=str(cert), NO_PROXY='localhost,127.0.0.1')

        def cli(*args, ok=True):
            result = subprocess.run([str(binary), 'skills', *map(str, args)], env=env,
                                    capture_output=True, text=True, timeout=35)
            assert (result.returncode == 0) is ok, (args, result.stdout, result.stderr)
            return result

        skill, store, webroot = root / 'skill', root / 'store', root / 'web'
        skill.mkdir()
        webroot.mkdir()
        (skill / 'SKILL.md').write_text('synthetic-feed-secret\n')
        source = root / 'rules.json'

        def pack(version, pattern='synthetic-feed-secret'):
            timestamp = int(time.time())
            source.write_text(json.dumps({
                'format': 'symbi-clawhavoc-pack-v1', 'version': version,
                'semantics_version': '2', 'issued_at': timestamp - 1,
                'expires_at': timestamp + 3600,
                'rules': [{'id': 'fixture-deny', 'pattern': pattern, 'severity': 'High',
                           'message': 'Synthetic feed rule', 'source': 'shipping CLI fixture'}],
            }))
            output = root / f'signed-{version}-{time.time_ns()}.json'
            cli('rules', 'sign', '--input', source, '--key', private, '--output', output)
            return output.read_bytes()

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0),
                   functools.partial(Handler, directory=str(webroot)))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, tls_key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            url = f'https://localhost:{server.server_port}/pack.json'
            cli('rules', 'init', '--store', store, '--url', url, '--public-key', public)
            cli('rules', 'init', '--store', store, '--url', url, '--public-key', public, ok=False)
            cli('rules', 'status', '--store', store, ok=False)
            cli('scan', skill, '--rules-dir', store, '--json', ok=False)
            first = pack(1)
            (webroot / 'pack.json').write_bytes(first)
            cli('rules', 'update', '--store', store)
            cli('rules', 'status', '--store', store)
            report = json.loads(cli('scan', skill, '--rules-dir', store, '--json', ok=False).stdout)
            assert report['findings'][0]['rule'] == 'pack:fixture-deny'
            assert report['identity']['ruleset_version'].endswith('+signed-1')
            cli('scan', skill, '--json')  # Explicit opt-in; bundled scanner is unchanged.
            cli('rules', 'update', '--store', store)  # Identical generation is idempotent.
            (webroot / 'pack.json').write_bytes(pack(2, 'different-secret'))
            cli('rules', 'update', '--store', store)
            assert (store / 'previous.json').read_bytes() == first
            cli('scan', skill, '--rules-dir', store, '--json')
            active = (store / 'active.json').read_bytes()
            for bad in [first, b'{}', pack(2, 'same-version-conflict')]:
                (webroot / 'pack.json').write_bytes(bad)
                cli('rules', 'update', '--store', store, ok=False)
                assert (store / 'active.json').read_bytes() == active
            (store / 'active.json').write_text('{}')
            cli('rules', 'status', '--store', store, ok=False)
            cli('scan', skill, '--rules-dir', store, '--json', ok=False)
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=5)
    print('Shipping signed-rule updater checks passed')


if __name__ == '__main__':
    main()
