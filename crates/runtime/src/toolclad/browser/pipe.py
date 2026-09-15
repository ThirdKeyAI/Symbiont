"""Carry private CDP bytes through a contained worker's standard streams.

The runtime chooses the executable and arguments. This adapter exposes no host
network listener; Chromium uses inherited descriptors 3 and 4 for CDP.
"""
import fcntl
import os
import subprocess
import sys
import threading

# Move every pipe end above 4 before assigning Chromium's fixed descriptors.
# This avoids accidentally overwriting the other pipe when descriptor numbers
# depend on the containing worker's inherited handles.
original = (*os.pipe(), *os.pipe())
read_output, browser_output, browser_input, write_input = [
    fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 10) for fd in original
]
for fd in original:
    os.close(fd)
os.dup2(browser_input, 3, inheritable=True)
os.dup2(browser_output, 4, inheritable=True)
child = subprocess.Popen(sys.argv[1:], stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL, stderr=sys.stderr, pass_fds=(3, 4), close_fds=True)
for fd in (3, 4, browser_input, browser_output):
    os.close(fd)


def copy_stream(source, target):
    try:
        while chunk := os.read(source, 8192):
            while chunk:
                written = os.write(target, chunk)
                if not written:
                    os._exit(1)
                chunk = chunk[written:]
    except OSError:
        os._exit(1)
    finally:
        os.close(target)


threading.Thread(target=copy_stream, args=(0, write_input), daemon=True).start()
# Output is drained before process status is returned. The independent worker
# supervisor bounds dead pipes, stalled children and detached descendants.
copy_stream(read_output, 1)
status = child.wait()
os._exit(status if status >= 0 else 128 - status)
