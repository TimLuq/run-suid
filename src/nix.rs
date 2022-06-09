use std::{path::{Path, PathBuf}, os::unix::{prelude::{MetadataExt, PermissionsExt, CommandExt}}, fs::Metadata, process::{Command, ExitCode}, collections::BTreeSet};

use parking_lot::Mutex;

use crate::{env::EnvTrait, RET_GENERIC_ERROR};

pub(crate) struct Nix {}

impl EnvTrait for Nix {
    #[inline]
    unsafe fn geteuid() -> u32 {
        libc::geteuid()
    }
    #[inline]
    unsafe fn getuid() -> u32 {
        libc::getuid()
    }
    #[inline]
    unsafe fn getegid() -> u32 {
        libc::getegid()
    }
    #[inline]
    fn file_owner(path: &Path) -> Result<(u32, Metadata, bool), std::io::Error> {
        file_owner(path)
    }
    #[inline]
    fn sibling_target(parent: &Path, file_name: &str) -> PathBuf {
        sibling_target(parent, file_name)
    }
    #[inline]
    fn prepare_command<'a, A: IntoIterator<Item = &'a str>>(command: &mut Command, args: A, opts: &super::Opts) {
        prepare_command(command, args, &opts)
    }
    #[inline]
    fn wait_for(child: Command, opts: super::Opts) -> ExitCode {
        wait_for(child, opts)
    }
}

const PERM_FILE_MASK: u32 = 0o4522;
const PERM_FILE_EXPECTED: u32 = 0o4500;
const PERM_DIR_MASK: u32 = 0o522;
const PERM_DIR_EXPECTED: u32 = 0o500;

fn file_owner(path: &Path) -> Result<(u32, Metadata, bool), std::io::Error> {
    let metadata = std::fs::metadata(path)?;
    let m = metadata.permissions().mode();
    let b = if metadata.is_dir() {
        m & PERM_DIR_MASK == PERM_DIR_EXPECTED
    } else if metadata.is_file() {
        m & PERM_FILE_MASK == PERM_FILE_EXPECTED
    } else {
        false
    };

    Ok((metadata.uid(), metadata, b))
}

fn sibling_target(parent: &Path, file_name: &str) -> PathBuf {
    let mut r = PathBuf::from(parent);
    if let Some(a) = file_name.split('.').last() {
        let pos = file_name.len() - a.len();
        if pos != 0 {
            r.push(format!("{}.run-suid.{}", &file_name[..(pos - 1)], &file_name[pos..]));
            return r;
        }
    }
    r.push(format!("{}.run-suid", file_name));
    r
}

static PATHS: &[&str] = &[
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

fn prepare_command<'a, A: IntoIterator<Item = &'a str>>(command: &mut Command, args: A, opts: &super::Opts) {
    command.args(args);
    command.env_clear();
    let cur_path: BTreeSet<_> = match std::env::var("PATH") {
        Ok(path) => path.split(':').map(str::to_owned).collect(),
        Err(_) => BTreeSet::new(),
    };
    let mut path = String::with_capacity(64);
    for p in PATHS {
        let p = *p;
        if cur_path.contains(p) {
            path.push_str(p);
            path.push(':');
        }
    }
    if !path.is_empty() {
        path.pop();
    } else {
        path.push_str("/bin");
    }
    // CommandExt::uid(command, opts.uid);
    // CommandExt::gid(command, opts.gid);
    command.env("PATH", path);
}

static COND: parking_lot::Condvar = parking_lot::Condvar::new();
static EXIT: parking_lot::Mutex<Option<ExitCode>> = parking_lot::Mutex::new(None);
static CAPTURED_SIGS_CONST: [i32; 20] = {
    use libc::*;

    [
        SIGABRT,
        SIGALRM,
        // SIGCHLD,
        SIGCONT,
        SIGFPE,
        SIGHUP,
        SIGILL,
        SIGINT,
        // SIGKILL,
        SIGPIPE,
        SIGPOLL,
        // SIGRTMIN..=SIGRTMAX,
        SIGQUIT,
        // SIGSEGV,
        SIGSTOP,
        SIGSYS,
        SIGTSTP,
        SIGTTIN,
        SIGTTOU,
        // SIGTRAP,
        SIGURG,
        SIGUSR1,
        SIGUSR2,
        SIGXCPU,
        SIGXFSZ,
    ]
};

static WAIT_FOR_PID: Mutex<(i32, i32)> = Mutex::new((0, 0));
static mut VERBOSE: bool = false;

fn signal_trap(signal: i32) {
    let mut exit = WAIT_FOR_PID.lock();
    let v = unsafe { std::ptr::read_volatile(&VERBOSE) };
    let (next_sig, pid) = &mut *exit;
    if *pid == 0 {
        if v {
            eprintln!("Verbose: queuing signal {}", signal);
        }
        *next_sig = signal;
    } else {
        if v {
            eprintln!("Verbose: sending signal {}", signal);
        }
        unsafe { libc::kill(*pid, signal) };
    }
    std::mem::drop(exit);
}

fn wait_for(mut child: Command, opts: super::Opts) -> ExitCode {
    let v = opts.verbose;
    unsafe { std::ptr::write_volatile(&mut VERBOSE, v) };
    std::thread::Builder::new()
        .name("wait-for-child".to_string())
        .stack_size(std::mem::size_of::<usize>() * 16)
        .spawn(move || {
            let v = opts.verbose;
            if v {
                use std::fmt::Write;
                let mut out = String::new();
                write!(out, "{:?}", child.get_program()).unwrap();
                for a in child.get_args() {
                    write!(out, " {:?}", a).unwrap();
                }
                write!(out, " {{ uid: {}, gid: {}, args: {{", opts.uid, opts.gid).unwrap();
                for a in child.get_envs() {
                    write!(out, " {:?}: {:?},", a.0, a.1).unwrap();
                }
                write!(out, "}} }}").unwrap();
                eprintln!("{}", out);
            }
            let mut child = match child.spawn() {
                Ok(child) => child,
                Err(e) => {
                    eprintln!("Unable to execute command: {}", e);
                    let mut exit = EXIT.lock();
                    *exit = Some(ExitCode::from(RET_GENERIC_ERROR));
                    COND.notify_all();
                    return;
                },
            };
            let cpid = child.id() as i32;
            {
                let mut exit = WAIT_FOR_PID.lock();
                let (next_sig, pid) = &mut *exit;
                *pid = cpid;
                if *next_sig != 0 {
                    if v {
                        eprintln!("Verbose: sending queued signal {:?}", child);
                    }
                    unsafe { libc::kill(*pid, *next_sig) };
                    *next_sig = 0;
                }
                std::mem::drop(exit)
            }
            if v {
                eprintln!("Verbose: waiting for child {:?}", cpid);
            }
            match child.wait() {
                Ok(r) => {
                    if v {
                        eprintln!("Verbose: child process exited {:?}", r);
                    }
                    let mut exit = EXIT.lock();
                    *exit = Some(ExitCode::from(r.code().unwrap_or(255) as u8));
                    COND.notify_all();
                }
                Err(e) => {
                    eprintln!("Unable to wait for child: {}", e);
                    let mut exit = EXIT.lock();
                    *exit = Some(ExitCode::from(RET_GENERIC_ERROR as u8));
                    COND.notify_all();
                }
            }
        }).unwrap();
    
    {
        let mut exit = EXIT.lock();
        if let Some(r) = exit.take() {
            return r;
        } else {
            if v {
                eprintln!("Verbose: registering signal handlers");
            }
            unsafe {
                use libc::*;
                // let range = (SIGRTMIN()..=SIGRTMAX()).collect::<SmallVec<[_; 32]>>();
                for signum in CAPTURED_SIGS_CONST.iter() {
                    if signal(*signum, signal_trap as usize) == SIG_IGN {
                        signal(*signum, SIG_IGN);
                    }
                }
            }
            loop {
                if v {
                    eprintln!("Verbose: waiting for child completion");
                }
                COND.wait(&mut exit);
                if let Some(r) = exit.take() {
                    return r;
                }
            }
        }
    }
}
