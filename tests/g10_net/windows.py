#!/usr/bin/env python3
"""G10 net A3: a real run of the net runtime on a Windows machine.

The Pruv side is compiled here (Rust backend; the LLVM shim links the same runtime text, see foreign_shim.py), then the
generated Rust and the unchanged A2 runtime drivers are copied over SSH to a fresh directory on the Windows host, built
there with rustc (MSVC target) and run there, one fresh working directory per case. Every case uses the 127.0.0.1
loopback only. The remote directory is removed at the end.

Cases: echo of bytes, half close, connect errors, listen, a small server, timeouts, broken pipe and the net_bytes copy.

usage: tests/g10_net/windows.py [--host USER@HOST] [--compiler rs|py] [--rustc PATH]
exit 0 when every case passes; the output lists each case.
"""
import os
import subprocess
import sys
import tempfile
import uuid

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RT = os.path.join(ROOT, "tests", "runtime")
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
SCP = ["scp", "-q", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
DEFAULT_RUSTC = r"%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustc.exe"

# portable A2/A3 drivers, run unchanged (the first entry of each case's flags matrix)
PORTABLE = [
    ("g10n_r01_echo_bytes", []),
    ("g10n_r03_half_close", []),
    ("g10n_r04_connect_errors", []),
    ("g10n_r05_listen", []),
    ("g10n_r06_serve", []),
    ("g10n_r08_timeouts", []),
    ("g10n_r09_broken_pipe", []),
    ("g10n_r17_copy_contract", []),
]


def compile_case(compiler, name, flags, out):
    src = os.path.join(RT, name + ".pruv")
    if compiler == "rs":
        argv = [os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc"), src, "-o", out] + flags
    else:
        argv = [os.path.join(ROOT, ".venv", "bin", "python"), os.path.join(ROOT, "pruvc", "pruvc.py"), src, "-o", out] + flags
    c = subprocess.run(argv, capture_output=True, text=True, timeout=600)
    return c.returncode == 0, c.stdout + c.stderr


def ssh(host, cmd, timeout=600):
    return subprocess.run(SSH + [host, cmd], capture_output=True, timeout=timeout)


def text(b):
    return b.decode("utf-8", "replace")


def main(argv):
    host, compiler, rustc = "aqw@172.30.1.59", "rs", DEFAULT_RUSTC
    i = 1
    while i < len(argv):
        if argv[i] in ("--host", "--compiler", "--rustc") and i + 1 < len(argv):
            if argv[i] == "--host":
                host = argv[i + 1]
            elif argv[i] == "--compiler":
                compiler = argv[i + 1]
            else:
                rustc = argv[i + 1]
            i += 2
        else:
            print(__doc__.strip().splitlines()[-2])
            return 2
    if compiler not in ("rs", "py"):
        print("--compiler must be rs or py")
        return 2

    rdir = "pruv_g10n_" + uuid.uuid4().hex[:12]
    failures = []
    with tempfile.TemporaryDirectory(prefix="g10n_") as work:
        files, built = [], []
        for name, flags in PORTABLE:
            gen = os.path.join(work, name + "_gen.rs")
            ok, out = compile_case(compiler, name, flags, gen)
            if not ok:
                failures.append((name, "pruvc: " + out[-400:]))
                continue
            main_rs = os.path.join(work, name + "_main.rs")
            with open(main_rs, "w") as fh:
                fh.write('#![allow(dead_code, unused_imports)]\n#[path = "%s_gen.rs"] mod sol;\n' % name)
                fh.write(open(os.path.join(RT, name + ".rs")).read())
            files += [gen, main_rs]
            built.append(name)

        r = ssh(host, 'mkdir "%%TEMP%%\\%s"' % rdir)
        if r.returncode != 0:
            print("cannot reach %s: %s" % (host, text(r.stderr)[-300:]))
            return 1
        r = ssh(host, 'echo %%TEMP%%\\%s' % rdir)
        remote = text(r.stdout).strip()
        try:
            up = subprocess.run(SCP + files + ["%s:%s/" % (host, remote.replace("\\", "/"))],
                                capture_output=True, timeout=600)
            if up.returncode != 0:
                print("scp failed: " + text(up.stderr)[-300:])
                return 1
            rc = '"%s"' % rustc
            for key in built:
                b = ssh(host, 'cd /d "%s" && %s -O --edition 2021 %s_main.rs -o %s.exe' % (remote, rc, key, key))
                if b.returncode != 0:
                    failures.append((key, "rustc: " + text(b.stdout + b.stderr)[-600:]))
                    continue
                cmd = 'cd /d "%s" && mkdir run_%s && cd run_%s && "%s\\%s.exe"' % (remote, key, key, remote, key)
                p = ssh(host, cmd, timeout=300)
                if p.returncode != 0:
                    failures.append((key, "exit %d: %s" % (p.returncode, text(p.stdout + p.stderr)[-600:])))
                else:
                    print("ok   %s" % key)
        finally:
            ssh(host, 'rmdir /s /q "%s"' % remote)
    for key, why in failures:
        print("FAIL %s: %s" % (key, why))
    print("%d/%d passed on %s (%s compiler, Rust backend)" % (len(PORTABLE) - len(failures), len(PORTABLE), host, compiler))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
