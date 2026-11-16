"""All clients connect before releasing the same deterministic 64 KiB payload."""
import concurrent.futures
import socket
import sys
import threading

payload=bytes((i*7+3)%251 for i in range(65536))
barrier=threading.Barrier(50,timeout=30)
def client(_):
    with socket.create_connection(('127.0.0.1',int(sys.argv[1])),timeout=120) as s:
        barrier.wait()
        s.sendall(payload)
        s.shutdown(socket.SHUT_WR)
        got=bytearray()
        while True:
            chunk=s.recv(65536)
            if not chunk: break
            got.extend(chunk)
        assert got==payload,(len(got),len(payload))
with concurrent.futures.ThreadPoolExecutor(max_workers=50) as pool:
    list(pool.map(client,range(50)))
print('CLIENTS 50 BYTES 3276800 EXACT_ECHO OK')
