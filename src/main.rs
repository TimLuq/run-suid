use std::{process::{Command, Stdio, ExitCode}, collections::VecDeque, ffi::OsStr, io::ErrorKind};

mod env;
use env::EnvTrait;
use smallvec::SmallVec;

#[cfg(unix)]
mod nix;
#[cfg(unix)]
type Env = nix::Nix;

#[cfg(not(unix))]
compile_error!("Unsupported platform");

const RET_GENERIC_ERROR: u8 = 32 | 1;
const RET_ENV_ERROR: u8 = 32 | 2;
const RET_NO_TARGET: u8 = 32 | 3;
const RET_OWNER_EXEC: u8 = 32 | 8 | 0;
const RET_PERM_EXEC: u8 = 32 | 8 | 1;
const RET_OWNER_PARENT: u8 = 32 | 8 | 2;
const RET_PERM_PARENT: u8 = 32 | 8 | 3;
const RET_OWNER_TARGET: u8 = 32 | 6;
const RET_PERM_TARGET: u8 = 32 | 6;

struct Opts {
    verbose: bool,
    dry_run: bool,
    uid: u32,
    gid: u32,
}

fn main() -> ExitCode {
    let mut args = std::env::args().collect::<VecDeque<_>>();
    let fname = args.pop_front().unwrap_or_default();
    let mut args_l = Vec::with_capacity(args.len());
    while let Some(f) = args.pop_front() {
        if f != "--" {
            args_l.push(f);
            continue;
        }
        break;
    }
    let args_l = args_l.iter().map(String::as_str).collect::<SmallVec<[_; 8]>>();
    let args = args.iter().map(String::as_str).collect::<SmallVec<[_; 8]>>();
    let gid = unsafe { Env::getegid() };
    
    if args_l.contains(&"--help") || args_l.contains(&"-h") {
        println!("Usage: {} [OPTIONS] [-- EXE_ARGS..]", fname);
        println!("  OPTIONS: ");
        println!("    -h    --help          Display this help text.");
        println!("    -v    --verbose       Display verbose runtime information.");
        println!("          --version       Display version information.");
        println!("          --dry-run       Don't actually run the target executable,");
        println!("                          only check that it would have run.");
        println!("  EXE_ARGS:");
        println!("    if specified, each argument will be passed to the executed subprocess.");
        if !args_l.contains(&"--help") {
            return ExitCode::SUCCESS;
        }
        println!("");
        println!(concat!(env!("CARGO_PKG_DESCRIPTION")));
        println!("");
    }
    
    if args_l.contains(&"--help") || args_l.contains(&"--version") {
        println!(concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")));
        println!(concat!("Author:   ", env!("CARGO_PKG_AUTHORS")));
        println!(concat!("Homepage: ", env!("CARGO_PKG_HOMEPAGE")));
        println!(concat!("License:  ", env!("CARGO_PKG_LICENSE")));

        return ExitCode::SUCCESS;
    }

    for arg in args_l.iter() {
        if !["-v", "--verbose", "--dry-run"].contains(arg) {
            eprintln!("Unexpected argument: {:?}", arg);
            return RET_GENERIC_ERROR.into();
        }
    }

    let cwd = match std::env::current_dir().and_then(|f| std::fs::canonicalize(f)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Unable to get the current directory: {}", e);
            return RET_GENERIC_ERROR.into();
        }
    };

    let exe = match std::env::current_exe().and_then(|f| std::fs::canonicalize(f)) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("Unable to find the name of the executable: {}", err);
            return RET_ENV_ERROR.into();
        }
    };
    let exe_uid = match Env::file_owner(&exe) {
        Ok((exe_uid, meta, true)) if meta.is_file() => exe_uid,
        Ok((_, _, true)) => {
            eprintln!("The executable must be a ... file: {:?}", exe);
            return RET_ENV_ERROR.into();
        }
        Ok((_, _, false)) => {
            eprintln!("The executable permissions must include the SUID bit as well as be writable by only the owning user: {:?}", exe);
            return RET_PERM_EXEC.into();
        }
        Err(err) => {
            eprintln!("Unable to find the owner of the executable: {}", err);
            return RET_ENV_ERROR.into();
        }
    };
    let exe_name = match exe.file_name().map(OsStr::to_str) {
        Some(Some(fname)) => fname,
        Some(None) => {
            eprintln!("Unable to read the name of the executable: {:?}", exe);
            return RET_ENV_ERROR.into();
        }
        None => {
            eprintln!("Unable to find the name of the executable: {:?}", exe);
            return RET_ENV_ERROR.into();
        }
    };

    let euid = unsafe { Env::geteuid() };

    if euid != exe_uid {
        eprintln!("You are not the owner of this executable.");
        return RET_OWNER_EXEC.into();
    }

    let parent = match exe.parent() {
        Some(a) => a,
        None => {
            eprintln!("Unable to find the parent directory of the executable: {}", exe.display());
            return RET_ENV_ERROR.into();
        }
    };
    let par_uid = match Env::file_owner(parent) {
        Ok((exe_uid, m, true)) if m.is_dir() => exe_uid,
        Ok((_, _, true)) => {
            eprintln!("The parent directory must be a ... directory: {:?}", parent);
            return RET_ENV_ERROR.into();
        }
        Ok((_, _, false)) => {
            eprintln!("The parent directory permissions must be writable by only the owning user: {:?}", parent);
            return RET_PERM_PARENT.into();
        }
        Err(err) => {
            eprintln!("Unable to find the owner of the parent directory: {}", err);
            return RET_ENV_ERROR.into();
        }
    };
    if euid != par_uid {
        eprintln!("The the owner of the parent directory is not the same as the executable.");
        return RET_OWNER_PARENT.into();
    }

    let target = Env::sibling_target(parent, exe_name);
    let tar_uid = match Env::file_owner(&target) {
        Ok((exe_uid, m, true)) if m.is_file() => exe_uid,
        Ok((_, _, true)) => {
            eprintln!("The target executable must be a file: {:?}", target);
            return RET_ENV_ERROR.into();
        }
        Ok((_, _, false)) => {
            eprintln!("The target executable permissions must include the SUID bit as well as be writable by only the owning user: {:?}", target);
            return RET_PERM_TARGET.into();
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("Unable to find the owner of the target executable {:?}: {}", target, err);
            return RET_NO_TARGET.into();
        }
        Err(err) => {
            eprintln!("Unable to find the owner of the target executable {:?}: {}", target, err);
            return RET_ENV_ERROR.into();
        }
    };
    if euid != 0 && euid != tar_uid {
        eprintln!("The the owner of the target executable is not the same as the executable.");
        return RET_OWNER_TARGET.into();
    }
    
    let opts = Opts {
        verbose: args_l.contains(&"--verbose") || args_l.contains(&"-v"),
        dry_run: args_l.contains(&"--dry-run"),
        uid: tar_uid,
        gid,
    };

    if opts.dry_run {
        use std::fmt::Write;
        let mut out = String::new();
        out.push_str("Dry run: would have succeeded in starting the process: ");
        write!(out, "{:?}", target).unwrap();
        for a in args {
            write!(out, " {:?}", a).unwrap();
        }
        println!("{}", out);
        return ExitCode::SUCCESS;
    }

    let mut command = Command::new(target);
    command.current_dir(cwd)
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .env_clear();
    Env::prepare_command(&mut command, args, &opts);

    Env::wait_for(command, opts)
}
