import hashlib
import json
import re
import selectors
import subprocess
from pathlib import Path

if not __debug__:
    raise RuntimeError('benchmark checks require Python assertions')

base = Path(__file__).resolve().parent
modes = ('reference', 'candidate')
suites = ('TLS_AES_128_GCM_SHA256', 'TLS_AES_256_GCM_SHA384', 'TLS_CHACHA20_POLY1305_SHA256')
expected = dict()
for line in (base/'wire-binaries.sha256').read_text().splitlines():
    digest, filename = line.split(maxsplit=1)
    path = Path(filename)
    assert path.parent == base/'bin' and path.name in modes
    with path.open('rb') as f:
        assert hashlib.file_digest(f, 'sha256').hexdigest() == digest
    expected[path.name] = digest
assert set(expected) == set(modes)
if len(set(expected.values())) != len(modes):
    raise RuntimeError('reference and candidate binaries have identical bytes')
for label in ('paired', 'wrong'):
    for extension in ('cert', 'key'):
        path = base/'fixture'/f'{label}.{extension}.pem'
        assert path.is_file() and not path.is_symlink()
opts = dict(stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
sandbox = ['/usr/bin/sandbox-exec', '-f', str(base/'loopback.sb')]


def command(mode, role, suite, size, retry_or_port, reject):
    return [*sandbox, str(base/'bin'/mode), role, str(base/'fixture'), str(suite), str(size),
            str(retry_or_port), 'wrong' if reject else 'normal']


def one(client_backend, server_backend, suite, size, retry, reject):
    server = subprocess.Popen(command(server_backend, 'server', suite, size, str(retry).lower(), reject), **opts)
    try:
        with selectors.DefaultSelector() as ready:
            ready.register(server.stdout, selectors.EVENT_READ)
            if not ready.select(timeout=4):
                raise RuntimeError('owned server did not publish readiness')
            announcement = server.stdout.readline().rstrip()
        match = re.fullmatch(r'READY ([0-9]{1,5})', announcement)
        if not match:
            raise RuntimeError(f'invalid server readiness {announcement!r}')
        port = int(match[1])
        assert 0 < port < 65536 and server.poll() is None
        client = subprocess.run(command(client_backend, 'client', suite, size, port, reject),
                                **opts, timeout=13)
        out, error = server.communicate(timeout=4)
        if client.returncode or server.returncode:
            raise RuntimeError(dict(client=client_backend, server=server_backend, suite=suites[suite],
                retry=retry, reject=reject, client_stdout=client.stdout, client_stderr=client.stderr,
                server_stdout=out, server_stderr=error))
        assert not error and not client.stderr
        lines = out.splitlines()
        if retry:
            assert lines.pop(0) == 'RETRY verified'
        result = dict(suite=suites[suite], client=client_backend, server=server_backend,
                      retry=retry, pin_rejected=reject)
        if reject:
            assert lines == ['REJECTED server'] and client.stdout.splitlines() == ['REJECTED client']
        else:
            value = re.fullmatch(r'READ (\d+) ([0-9.]+) ([0-9.]+)\n', client.stdout)
            assert value and int(value[1]) == size and lines == [f'SENT {size}']
            handshake, total = [float(value[i]) for i in (2,3)]
            assert 0 < handshake <= total < 10000
            result.update(bytes=size, handshake_ms=handshake, complete_ms=total)
        return result
    finally:
        if server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=3)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=3)


def record(row):
    print(json.dumps(row, sort_keys=True), flush=True)


for suite in range(len(suites)):
    for retry in (False, True):
        for client_mode in modes:
            for server_mode in modes:
                record(dict(stage='interop', **one(client_mode, server_mode, suite, 65537, retry, False)))
for client_mode in modes:
    for server_mode in modes:
        record(dict(stage='pin', **one(client_mode, server_mode, 0, 65537, False, True)))
for suite in range(len(suites)):
    for pair in range(6):
        order = modes if pair % 2 == 0 else tuple(reversed(modes))
        for mode in order:
            record(dict(stage='measured', pair=pair, **one(mode, mode, suite, 8 * 1024 * 1024, False, False)))
record(dict(stage='withdrawn', binary_sha256=expected))
