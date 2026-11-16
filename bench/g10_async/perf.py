"""G10 async measurement: 50 concurrent clients, 64 KiB exact echo, no target."""
import sys
sys.dont_write_bytecode = True
import argparse
import importlib.util
import json
import os
import platform
from pathlib import Path
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
WORK = HERE / '.work'
WORK.mkdir(exist_ok=True)
os.environ['TMPDIR'] = str(WORK)
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
os.environ['CLANG_MODULE_CACHE_PATH'] = str(WORK / 'clang-cache')
spec = importlib.util.spec_from_file_location('net_q', ROOT / 'bench/g10_net/run.py')
G = importlib.util.module_from_spec(spec)
spec.loader.exec_module(G)
G.WORK = WORK
os.environ['TMPDIR'] = str(WORK)
os.environ['CLANG_MODULE_CACHE_PATH'] = str(WORK / 'clang-cache')
tempfile.tempdir = str(WORK)
G.G2.COMPILERS['py'] = [os.environ.get('PRUV_PYTHON', '/Users/spica4345/Projects/pruv/.venv/bin/python'), str(ROOT / 'pruvc/pruvc.py')]

def measure(exe, mode):
    server = subprocess.Popen([str(exe), str(mode)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    client = None
    try:
        ready = server.stdout.readline().strip()
        if not ready.removeprefix('PORT ').isdigit():
            err = server.communicate(timeout=10)[1]
            return {'status': 'permission_error' if 'Permission denied' in err or 'Operation not permitted' in err else 'run_error', 'stdout':ready, 'stderr':err}
        start = time.perf_counter_ns()
        client = subprocess.Popen([sys.executable, str(HERE / 'perf/client.py'), ready.removeprefix('PORT ')], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        rss = threads = samples = 0
        while server.poll() is None and time.perf_counter_ns() - start < 240_000_000_000:
            # macOS `ps -M` ignores -o, so RSS and the thread count are read separately
            pr = subprocess.run(['ps', '-o', 'rss=', '-p', str(server.pid)], capture_output=True, text=True)
            if platform.system() == 'Darwin':
                pt = subprocess.run(['ps', '-M', '-p', str(server.pid)], capture_output=True, text=True)
                n = len(pt.stdout.splitlines()) - 1
            else:
                pt = subprocess.run(['ps', '-o', 'nlwp=', '-p', str(server.pid)], capture_output=True, text=True)
                n = int(pt.stdout.strip() or 0)
            if pr.returncode == 0 and pt.returncode == 0 and pr.stdout.strip() and n > 0:
                rss=max(rss,int(pr.stdout.strip())*1024); threads=max(threads,n); samples+=1
            time.sleep(.01)
        elapsed = time.perf_counter_ns() - start
        if server.poll() is None: server.kill()
        out,err = server.communicate(timeout=10)
        try: co,ce = client.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            client.kill(); co,ce=client.communicate()
        return {'status':'ok' if server.returncode == 0 and client.returncode == 0 else 'run_error',
                'elapsed_ns':elapsed, 'max_rss_bytes_sampled':rss if samples else None,
                'max_live_threads_sampled':threads if samples else None, 'samples':samples,
                'server_rc':server.returncode,'client_rc':client.returncode,'stdout':out+co,'stderr':err+ce}
    finally:
        for process in (client,server):
            if process is not None and process.poll() is None: process.kill(); process.wait()

def main():
    p=argparse.ArgumentParser(__doc__)
    p.add_argument('out',nargs='?',type=Path)
    p.add_argument('--compiler',choices=['py','rs','both'],default='both')
    p.add_argument('--backend',choices=['rust','llvm','both'],default='both')
    p.add_argument('--repeat',type=int,default=1)
    a=p.parse_args()
    if a.repeat<1: p.error('--repeat must be positive')
    failed=False
    for compiler in (['py','rs'] if a.compiler=='both' else [a.compiler]):
        rows={}
        for backend in (['rust','llvm'] if a.backend=='both' else [a.backend]):
            with tempfile.TemporaryDirectory(dir=WORK) as td:
                wd=Path(td)
                c=G.G2.compile_pruv(compiler,str(HERE/'perf/echo.pruv'),str(wd),backend)
                if c['rc']:
                    rows[backend]=dict(c,status='pruv_error'); print(compiler,backend,'pruv_error',c,flush=True); failed=True; continue
                exe,error=G.build(c,wd,(HERE/'perf/host.rs').read_text())
                native=wd/'native'
                build=G.run([G.G2.RUSTC,'-O','--edition=2021',HERE/'perf/threaded.rs','-o',native])
                if error or build['rc']:
                    rows[backend]={'status':'compile_error','error':error,'native':build}; print(rows[backend],flush=True); failed=True; continue
                modes={}
                for mode,name in [(0,'per_byte'),(1,'bulk_copy')]:
                    modes[name]={side:[measure(binary,mode) for _ in range(a.repeat)] for side,binary in [('pruv',exe),('rust',native)]}
                    for side,runs in modes[name].items():
                        for row in runs:
                            print(compiler,backend,name,side,json.dumps(row),flush=True); failed |= row['status']!='ok'
                statuses=[r['status'] for sides in modes.values() for runs in sides.values() for r in runs]
                status='ok' if all(s=='ok' for s in statuses) else 'permission_error' if all(s=='permission_error' for s in statuses) else 'run_error'
                rows[backend]={'status':status,'modes':modes}
        out=a.out or HERE/'results'/f'perf_{compiler}.json'
        if a.out and a.compiler=='both': out=out.with_name(out.stem+'_'+compiler+out.suffix)
        out.parent.mkdir(parents=True,exist_ok=True)
        out.write_text(json.dumps({'compiler':compiler,'verified':True,'clients':50,'bytes_per_client':65536,'repeat':a.repeat,'platform':platform.platform(),'tools':G.G2.COMPILERS,'rustc':G.run([G.G2.RUSTC,'--version']),'method':'Server wall time includes client startup; RSS and live threads are sampled at 10ms plus ps overhead. Client memory excluded. per_byte reads and writes one byte through ev_byte (the Rust server reads one byte per call and writes one byte per call into a 4 KiB BufWriter, as the Pruv runtime writer coalesces queued writes); bulk_copy copies and writes each chunk. Exact payload and EOF checked. No target.', 'rows':rows},indent=2))
    return int(failed)

if __name__=='__main__': sys.exit(main())
