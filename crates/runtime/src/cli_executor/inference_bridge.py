"""Run a CLI with a loopback adapter to the private inference socket."""
import http.client
import http.server
import os
import select
import socket
import socketserver
import subprocess
import sys
import threading

CHANNEL = '/opt/symbi-broker/inference.sock'
LIMIT = 1024 * 1024
VSOCK = len(sys.argv) > 1 and sys.argv[1] == '--vsock'
INHERITED = len(sys.argv) > 1 and sys.argv[1] == '--inherited'
if VSOCK or INHERITED:
    del sys.argv[1]

# The native CLI must exec in the original process: its Landlock metadata rule
# pins that process's maps inode. Fork the adapters before starting any threads.
# A pidfd ends their lifetime even if the CLI aborts without closing stdio.
if INHERITED:
    parent = os.pidfd_open(os.getpid())
    ready_read, ready_write = os.pipe()
    if os.fork():
        os.close(parent)
        os.close(ready_write)
        ready = os.read(ready_read, 1)
        os.close(ready_read)
        for name in ('SYMBI_TOOLS_FD', 'SYMBI_INFERENCE_FD'):
            os.close(int(os.environ.pop(name)))
        if ready != b'1':
            raise RuntimeError('private broker adapters failed to start')
        os.execvpe(sys.argv[1], sys.argv[1:], os.environ)
    os.close(ready_read)

# Native workers receive two already-connected capabilities. Child subprocesses
# use only private loopback; broker descriptors stay in this adapter process.
upstream_lock = threading.Lock()
if INHERITED:
    tools = socket.socket(fileno=int(os.environ.pop('SYMBI_TOOLS_FD')))
    inference = socket.socket(fileno=int(os.environ.pop('SYMBI_INFERENCE_FD')))
    tools.set_inheritable(False)
    inference.set_inheritable(False)
    inference.settimeout(120)
    tool_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    tool_listener.bind(('127.0.0.1', 8766))
    tool_listener.listen(1)

    def relay_tools():
        client, _ = tool_listener.accept()
        tool_listener.close()
        def send_tools():
            try:
                while chunk := client.recv(8192):
                    tools.sendall(chunk)
                tools.shutdown(socket.SHUT_WR)
            except OSError:
                tools.close()
        threading.Thread(target=send_tools, daemon=True).start()
        try:
            while chunk := tools.recv(8192):
                client.sendall(chunk)
        finally:
            client.close()
            tools.close()

    threading.Thread(target=relay_tools, daemon=True).start()


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
            if INHERITED:
                with upstream_lock:
                    header = f'POST {self.path} HTTP/1.1\r\nHost: private-inference\r\nContent-Length: {length}\r\nConnection: keep-alive\r\n\r\n'
                    inference.sendall(header.encode('ascii') + body)
                    response = http.client.HTTPResponse(inference)
                    response.begin()
                    size = int(response.getheader('Content-Length', '-1'))
                    if not 0 <= size <= 4 * LIMIT:
                        raise ValueError('invalid broker response length')
                    output = response.read(size + 1)
                    if len(output) != size:
                        raise ValueError('incomplete broker response')
                    self.send_response(response.status)
                    self.send_header('Content-Type', response.getheader('Content-Type', 'application/json'))
                    self.send_header('Content-Length', str(size))
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    self.wfile.write(output)
                    response.close()
                self.close_connection = True
                return
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
        except (OSError, ValueError, UnicodeError, http.client.HTTPException):
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
if INHERITED:
    os.write(ready_write, b'1')
    os.close(ready_write)
    select.select([parent], [], [])
    os._exit(0)
try:
    result = subprocess.run(sys.argv[1:], check=False)
finally:
    server.shutdown()
    server.server_close()
os._exit(result.returncode if result.returncode >= 0 else 128 - result.returncode)
