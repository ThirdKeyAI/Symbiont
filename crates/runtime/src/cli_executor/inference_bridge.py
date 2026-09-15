"""Run a CLI with a loopback adapter to the private inference socket."""
import http.server
import os
import socket
import socketserver
import subprocess
import sys
import threading

CHANNEL = '/opt/symbi-broker/inference.sock'
LIMIT = 1024 * 1024
VSOCK = len(sys.argv) > 1 and sys.argv[1] == '--vsock'
if VSOCK:
    del sys.argv[1]


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_POST(self):
        self.connection.settimeout(120)
        lengths = self.headers.get_all('Content-Length', [])
        try:
            if len(lengths) != 1 or self.headers.get('Transfer-Encoding') or self.headers.get('Expect'):
                raise ValueError('unsupported framing')
            length = int(lengths[0])
            if not 0 < length <= LIMIT or len(self.path) > 128:
                raise ValueError('request exceeds limit')
            body = self.rfile.read(length)
            if len(body) != length:
                raise ValueError('incomplete request')
            with socket.socket(socket.AF_VSOCK if VSOCK else socket.AF_UNIX, socket.SOCK_STREAM) as upstream:
                upstream.settimeout(120)
                upstream.connect((2, 4052) if VSOCK else CHANNEL)
                header = f'POST {self.path} HTTP/1.1\r\nHost: private-inference\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n'
                upstream.sendall(header.encode('ascii') + body)
                # The runtime reconstructs and bounds the response. Forward it
                # intact, with no worker-supplied headers or provider credentials.
                while chunk := upstream.recv(8192):
                    self.connection.sendall(chunk)
            self.close_connection = True
        except (OSError, ValueError, UnicodeError):
            self.close_connection = True


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    request_queue_size = 8
    slots = threading.BoundedSemaphore(8)

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        super().process_request(request, address)

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()


server = Server(('127.0.0.1', 8765), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    result = subprocess.run(sys.argv[1:], check=False)
finally:
    server.shutdown()
    server.server_close()
os._exit(result.returncode if result.returncode >= 0 else 128 - result.returncode)
