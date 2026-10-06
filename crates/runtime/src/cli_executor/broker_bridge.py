"""Bounded-buffer stdio transport for a private runtime MCP broker."""
import os
import socket
import sys
import threading

vsock = sys.argv[1] == 'vsock:2:4051'
tcp = sys.argv[1] == 'tcp:127.0.0.1:8766'
connection = socket.socket(socket.AF_VSOCK if vsock else (socket.AF_INET if tcp else socket.AF_UNIX), socket.SOCK_STREAM)
connection.settimeout(10)
connection.connect((2, 4051) if vsock else (('127.0.0.1', 8766) if tcp else sys.argv[1]))
connection.settimeout(None)


def send_input():
    complete = True
    try:
        while chunk := os.read(0, 8192):
            connection.sendall(chunk)
            complete = chunk.endswith(b'\n')
        if not complete:
            os._exit(1)
        # Firecracker does not translate guest SHUT_WR into host socket EOF.
        # End this private connection in-band after all complete requests;
        # the broker drains their responses before closing its socket.
        connection.sendall(b'{"jsonrpc":"2.0","method":"notifications/symbi/close"}\n')
    except OSError:
        os._exit(1)
    finally:
        try:
            connection.shutdown(socket.SHUT_WR)
        except OSError:
            pass


threading.Thread(target=send_input, daemon=True).start()
try:
    while chunk := connection.recv(8192):
        while chunk:
            chunk = chunk[os.write(1, chunk):]
except OSError:
    os._exit(1)
os._exit(0)
