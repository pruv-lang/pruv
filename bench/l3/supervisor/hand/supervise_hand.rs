#[allow(dead_code)]
mod core_hand;

use core_hand::Tail;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

type Output = io::Result<(usize, Vec<u8>)>;

fn read_pipe(mut pipe: impl Read, stream: usize, sender: SyncSender<Output>) {
    let mut buf = [0; 65536];
    loop {
        let item = match pipe.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => Ok((stream, buf[..n].to_vec())),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = sender.send(Err(e));
                return;
            }
        };
        if sender.send(item).is_err() {
            return;
        }
    }
}

fn stopped(socket: &mut TcpStream) -> i64 {
    loop {
        match socket.read(&mut [0]) {
            Ok(0) => return 2,
            Ok(_) => return 1,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return 0,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return 2,
        }
    }
}

// One chunk into the capped file: the prefix that fits, counted only when the write succeeds (Pruv
// `capfile::write`; split out of the receive loop in step 4.2 so the two sides compare per function).
fn write(file: &mut Option<File>, packet: &mut [i64], file_limit: i64, bytes: &[u8]) {
    if let Some(output) = file {
        let n = (file_limit - packet[10]).min(bytes.len() as i64);
        packet[6] |= i64::from(n < bytes.len() as i64);
        if n != 0 {
            if output.write_all(&bytes[..n as usize]).is_ok() {
                packet[10] += n;
            } else {
                packet[7] = 2;
                *file = None;
            }
        }
    }
}

pub fn supervise(
    shell: &str, argv: &str, cwd: &str, out: &str,
    file_limit: i64, tail_chars: i64, ctl: i64,
) -> Vec<i64> {
    let mut packet = vec![0; 12];
    if !(1..=65536).contains(&tail_chars)
        || !(0..=(1 << 40)).contains(&file_limit)
        || !(1..=65535).contains(&ctl)
        || argv.len() > 1048576
        || (!argv.is_empty() && !argv.ends_with('\0'))
    {
        packet[0] = 1;
        packet[1] = 5;
        return packet;
    }
    let Ok(mut socket) = TcpStream::connect(("127.0.0.1", ctl as u16)) else {
        packet[1] = 4;
        return packet;
    };
    let mut file = File::create(out).ok();
    packet[7] = i64::from(file.is_none());
    packet[8] = i64::from(file.is_some());

    let mut command = Command::new(shell);
    command.args(argv.split_terminator('\0'));
    if !cwd.is_empty() {
        command.current_dir(cwd);
    }
    let mut child = match command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(err) => {
            packet[1] = 2;
            packet[2] = match err.kind() {
                _ if shell.is_empty() || err.kind() == io::ErrorKind::InvalidInput => 1,
                io::ErrorKind::PermissionDenied => 3,
                _ => 2,
            };
            return packet;
        }
    };
    packet[1] = 1;
    if socket.write_all(&[1]).is_err() || socket.set_nonblocking(true).is_err() {
        packet[4] = 2;
        let _ = child.kill();
        packet[5] = i64::from(child.wait().is_ok());
        return packet;
    }

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut tail = Tail::new(tail_chars as usize);
    let interval = Duration::from_millis(2);
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(2);
        let stdout_sender = sender.clone();
        scope.spawn(move || read_pipe(stdout, 0, stdout_sender));
        scope.spawn(move || read_pipe(stderr, 1, sender));

        loop {
            if packet[4] == 0 {
                packet[4] = stopped(&mut socket);
                if packet[4] != 0 {
                    let _ = child.kill();
                }
            }
            match receiver.recv_timeout(interval) {
                Ok(Ok((stream, bytes))) => {
                    packet[9] += bytes.len() as i64;
                    write(&mut file, &mut packet, file_limit, &bytes);
                    tail.push(stream, &bytes);
                }
                Ok(Err(_)) => {
                    packet[1] = 3;
                    let _ = child.kill();
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // Pipes can close while the child is still running.
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            packet[5] = 1;
                            if packet[4] == 0 {
                                packet[2] = i64::from(status.code().unwrap_or_else(|| -status.signal().unwrap()));
                                packet[3] = 1;
                            }
                            break;
                        }
                        Ok(None) => thread::sleep(interval),
                        Err(_) => {
                            packet[1] = 3;
                            let _ = child.kill();
                            break;
                        }
                    }
                }
            }
        }
    });
    drop(file);
    if packet[1] == 3 {
        let _ = child.wait();
        packet[4] = 0;
        packet[9] = 0;
        packet[10] = 0;
    } else {
        let bytes = tail.finish();
        packet[11] = bytes.len() as i64;
        packet.extend(bytes.into_iter().map(i64::from));
    }
    packet
}
