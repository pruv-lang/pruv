#!/usr/bin/env python3
"""G10 proc A3 gates 8 and 9: a real run of the proc runtime on a Windows machine.

The Pruv side is compiled here (Rust backend; the LLVM shim links the same runtime text, see foreign_shim.py), then the
generated Rust, the host drivers and the fixture child tests/g10_proc/child.rs are copied over SSH to a fresh
directory on the Windows host, built there with rustc (MSVC target) and run there, one fresh working directory per
case. The remote directory is removed at the end.

Cases: the portable A2 runtime drivers (argument quoting and Unicode, spawn errors, stdin EOF, chunks, both outputs
under flooding, read_all, the proc_bytes copy) run unchanged; three Windows drivers in this file replace the Unix
ones: the working directory (no canonicalize, which adds a \\\\?\\ prefix on Windows), exit codes and kill (kill is
TerminateProcess with code 1; a negative exit is its unsigned 32-bit code) and batch files (gate 9: .bat/.cmd given
directly, in any case and with a trailing dot or space, are refused with error 1; a bare name never resolves to a batch
file, so it is error 2; no batch file ever runs, which marker files show).

usage: tests/g10_proc/windows.py [--host USER@HOST] [--compiler rs|py] [--rustc PATH]
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

# portable A2 drivers, run unchanged (the first entry of each case's flags matrix)
PORTABLE = [
    ("g10p_r01_args", []),
    ("g10p_r03_spawn_errors", []),
    ("g10p_r04_stdin_echo", []),
    ("g10p_r06_chunks", ["--no-verify"]),
    ("g10p_r07_stderr_flood", ["--no-verify"]),
    ("g10p_r08_flood_both", ["--no-verify"]),
    ("g10p_r09_read_all_nodup", ["--no-verify"]),
    ("g10p_r21_copy_contract", []),
]

CWD_DRIVER = r'''
fn line(p: &std::path::Path) -> Vec<u8> {
    let mut v = p.to_string_lossy().as_bytes().to_vec();
    v.push(b'\n');
    v
}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    let start = std::env::current_dir().unwrap();
    std::fs::create_dir("sub dir").unwrap();
    std::fs::create_dir("h\u{e9}llo \u{65e5}\u{672c}").unwrap();
    assert_eq!(sol::child_cwd(&child, "", &line(&start)), 0);
    assert_eq!(sol::child_cwd(&child, "sub dir", &line(&start.join("sub dir"))), 0);
    let abs = start.join("sub dir");
    assert_eq!(sol::child_cwd(&child, abs.to_str().unwrap(), &line(&abs)), 0);
    let uni = start.join("h\u{e9}llo \u{65e5}\u{672c}");
    assert_eq!(sol::child_cwd(&child, "h\u{e9}llo \u{65e5}\u{672c}", &line(&uni)), 0);
    assert_eq!(sol::child_cwd(&child, "", &line(&start)), 0);
    assert_eq!(std::env::current_dir().unwrap(), start);
}
'''

EXIT_DRIVER = r'''
fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(sol::run_exit(&child, "0"), 0);
    assert_eq!(sol::run_exit(&child, "7"), 7);
    assert_eq!(sol::run_exit(&child, "255"), 255);
    // exit(-1) is the unsigned 32-bit code 0xFFFFFFFF on Windows, never a negative (signal-like) value
    assert_eq!(sol::run_exit(&child, "4294967295"), 4294967295);
    // kill of a running child: TerminateProcess with exit code 1 (never 0, never 9, the code of an unkilled `hold`)
    assert_eq!(sol::kill_running(&child), 1);
    // kill of a child that exited or is exiting: a success, its own code 5 or the 1 of a kill that won the race
    for _ in 0..5 {
        let e = sol::kill_exited(&child);
        assert!(e == 5 || e == 1, "kill of an exited child gave {}", e);
    }
    assert_eq!(sol::failed_wait("C:\\no\\such\\dir\\pruv-missing-program.exe"), 2);
    assert_eq!(sol::failed_kill("C:\\no\\such\\dir\\pruv-missing-program.exe"), 2);
    assert_eq!(sol::failed_wait(""), 1);
    assert_eq!(sol::failed_kill(""), 1);
}
'''

BATCH_DRIVER = r'''
fn bat(path: &std::path::Path, mark: &str) {
    // a batch file that leaves a marker next to itself when it runs
    std::fs::write(path, format!("@echo off\r\necho ran> \"%~dp0{}\"\r\n", mark)).unwrap();
}

fn main() {
    let start = std::env::current_dir().unwrap();
    bat(&start.join("x.bat"), "ran_x.txt");
    bat(&start.join("w.cmd"), "ran_w.txt");
    std::fs::create_dir("pbin").unwrap();
    bat(&start.join("pbin").join("y.bat"), "ran_y.txt");
    bat(&start.join("pbin").join("z.cmd"), "ran_z.txt");
    let abs = start.join("x.bat");
    let direct: Vec<String> = vec![
        "x.bat".into(), "X.BAT".into(), "x.Bat".into(), "w.cmd".into(), "W.CMD".into(), ".\\x.bat".into(),
        "x.bat.".into(), "x.bat ".into(), "x.bat. .".into(), "w.cmd..".into(), abs.to_string_lossy().into_owned(),
    ];
    for prog in &direct {
        // error 1 (bad input), consumed by wait with Err(1): 1 * 10 + 1
        assert_eq!(sol::spawn_code(prog, "", "a"), 11, "batch file {:?}", prog);
    }
    // found by search: a bare name next to the parent and on PATH resolves to no batch file (the OS refused: 22)
    let old = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(start.join("pbin").as_os_str());
    path.push(";");
    path.push(&old);
    std::env::set_var("PATH", &path);
    for prog in ["x", "w", "y", "z"] {
        assert_eq!(sol::spawn_code(prog, "", "a"), 22, "bare name {:?}", prog);
    }
    std::env::set_var("PATH", &old);
    std::thread::sleep(std::time::Duration::from_millis(500));
    for m in ["ran_x.txt", "ran_w.txt", "pbin\\ran_y.txt", "pbin\\ran_z.txt"] {
        assert!(!start.join(m).exists(), "a batch file ran: {}", m);
    }
}
'''

WINDOWS = [
    ("g10p_r02_cwd", [], CWD_DRIVER, "win_cwd"),
    ("g10p_r10_exit_kill", [], EXIT_DRIVER, "win_exit_kill"),
    ("g10p_r03_spawn_errors", [], BATCH_DRIVER, "win_batch"),
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

    cases = [(n, f, open(os.path.join(RT, n + ".rs")).read(), n) for n, f in PORTABLE]
    cases += WINDOWS

    rdir = "pruv_g10w_" + uuid.uuid4().hex[:12]
    failures = []
    with tempfile.TemporaryDirectory(prefix="g10w_") as work:
        files = [os.path.join(ROOT, "tests", "g10_proc", "child.rs")]
        built = []
        for name, flags, driver, key in cases:
            gen = os.path.join(work, key + "_gen.rs")
            ok, out = compile_case(compiler, name, flags, gen)
            if not ok:
                failures.append((key, "pruvc: " + out[-400:]))
                continue
            main_rs = os.path.join(work, key + "_main.rs")
            with open(main_rs, "w") as fh:
                fh.write('#![allow(dead_code, unused_imports)]\n#[path = "%s_gen.rs"] mod sol;\n' % key)
                fh.write(driver)
            files += [gen, main_rs]
            built.append(key)

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
            b = ssh(host, 'cd /d "%s" && %s -O --edition 2021 child.rs -o child.exe' % (remote, rc))
            if b.returncode != 0:
                print("child build failed: " + text(b.stdout + b.stderr)[-600:])
                return 1
            for key in built:
                b = ssh(host, 'cd /d "%s" && %s -O --edition 2021 %s_main.rs -o %s.exe' % (remote, rc, key, key))
                if b.returncode != 0:
                    failures.append((key, "rustc: " + text(b.stdout + b.stderr)[-600:]))
                    continue
                cmd = ('cd /d "%s" && mkdir run_%s && cd run_%s && set "PRUV_PROC_CHILD=%s\\child.exe" && "%s\\%s.exe"'
                       % (remote, key, key, remote, remote, key))
                p = ssh(host, cmd, timeout=300)
                if p.returncode != 0:
                    failures.append((key, "exit %d: %s" % (p.returncode, text(p.stdout + p.stderr)[-600:])))
                else:
                    print("ok   %s" % key)
        finally:
            ssh(host, 'rmdir /s /q "%s"' % remote)
    for key, why in failures:
        print("FAIL %s: %s" % (key, why))
    print("%d/%d passed on %s (%s compiler, Rust backend)" % (len(cases) - len(failures), len(cases), host, compiler))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
