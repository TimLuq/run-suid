use std::{process::{Command, Stdio}, collections::VecDeque, ffi::OsStr, io::ErrorKind};

mod env;
use env::EnvTrait;

#[cfg(unix)]
mod nix;

#[cfg(unix)]
use nix::*;

#[cfg(unix)]
type Env = nix::Nix;

#[cfg(not(unix))]
compile_error!("Unsupported platform");

const RET_GENERIC_ERROR: i32 = 32 | 1;
const RET_ENV_ERROR: i32 = 32 | 2;
const RET_NO_TARGET: i32 = 32 | 3;
const RET_OWNER_EXEC: i32 = 32 | 8 | 0;
const RET_PERM_EXEC: i32 = 32 | 8 | 1;
const RET_OWNER_PARENT: i32 = 32 | 8 | 2;
const RET_PERM_PARENT: i32 = 32 | 8 | 3;
const RET_OWNER_TARGET: i32 = 32 | 6;
const RET_PERM_TARGET: i32 = 32 | 6;

fn main() {
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
    let args_l = args_l.iter().map(String::as_str).collect::<Vec<_>>();
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let gid = unsafe { Env::getegid() };
    
    if args_l.contains(&"--help") || args_l.contains(&"-h") {
        println!("Usage: {} [OPTIONS] [-- EXE_ARGS..]", fname);
        println!("  OPTIONS: ");
        println!("    -h    --help          Display this help text.");
        println!("    -v    --version       Display version information.");
        println!("          --dry-run       Don't actually run the target executable,");
        println!("                          only check that it would have run.");
        println!("  EXE_ARGS:");
        println!("    if specified, each argument will be passed to the executed subprocess.");
        if !args_l.contains(&"--help") {
            std::process::exit(0);
        }
        println!("");
        println!(concat!(env!("CARGO_PKG_DESCRIPTION")));
        println!("");
    }
    
    if args_l.contains(&"--help") || args_l.contains(&"--version") || args_l.contains(&"-v") {
        println!(concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")));
        println!(concat!("Author:   ", env!("CARGO_PKG_AUTHORS")));
        println!(concat!("Homepage: ", env!("CARGO_PKG_HOMEPAGE")));
        println!(concat!("License:  ", env!("CARGO_PKG_LICENSE")));

        std::process::exit(0);
    }

    let cwd = match std::env::current_dir().and_then(|f| std::fs::canonicalize(f)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Unable to get the current directory: {}", e);
            std::process::exit(RET_GENERIC_ERROR);
        }
    };

    let exe = match std::env::current_exe().and_then(|f| std::fs::canonicalize(f)) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("Unable to find the name of the executable: {}", err);
            std::process::exit(RET_ENV_ERROR);
        }
    };
    let exe_uid = match Env::file_owner(&exe) {
        Ok((exe_uid, meta, true)) if meta.is_file() => exe_uid,
        Ok((_, _, true)) => {
            eprintln!("The executable must be a ... file: {:?}", exe);
            std::process::exit(RET_ENV_ERROR);
        }
        Ok((_, _, false)) => {
            eprintln!("The executable permissions must include the SUID bit as well as be writable by only the owning user: {:?}", exe);
            std::process::exit(RET_PERM_EXEC);
        }
        Err(err) => {
            eprintln!("Unable to find the owner of the executable: {}", err);
            std::process::exit(RET_ENV_ERROR);
        }
    };
    let exe_name = match exe.file_name().map(OsStr::to_str) {
        Some(Some(fname)) => fname,
        Some(None) => {
            eprintln!("Unable to read the name of the executable: {:?}", exe);
            std::process::exit(RET_ENV_ERROR);
        }
        None => {
            eprintln!("Unable to find the name of the executable: {:?}", exe);
            std::process::exit(RET_ENV_ERROR);
        }
    };

    let euid = unsafe { geteuid() };

    if euid != exe_uid {
        eprintln!("You are not the owner of this executable.");
        std::process::exit(RET_OWNER_EXEC);
    }

    let parent = match exe.parent() {
        Some(a) => a,
        None => {
            eprintln!("Unable to find the parent directory of the executable: {}", exe.display());
            std::process::exit(RET_ENV_ERROR);
        }
    };
    let par_uid = match Env::file_owner(parent) {
        Ok((exe_uid, m, true)) if m.is_dir() => exe_uid,
        Ok((_, _, true)) => {
            eprintln!("The parent directory must be a ... directory: {:?}", parent);
            std::process::exit(RET_ENV_ERROR);
        }
        Ok((_, _, false)) => {
            eprintln!("The parent directory permissions must be writable by only the owning user: {:?}", parent);
            std::process::exit(RET_PERM_PARENT);
        }
        Err(err) => {
            eprintln!("Unable to find the owner of the parent directory: {}", err);
            std::process::exit(RET_ENV_ERROR);
        }
    };
    if euid != par_uid {
        eprintln!("The the owner of the parent directory is not the same as the executable.");
        std::process::exit(RET_OWNER_PARENT);
    }

    let target = Env::sibling_target(parent, exe_name);
    let tar_uid = match Env::file_owner(&target) {
        Ok((exe_uid, m, true)) if m.is_file() => exe_uid,
        Ok((_, _, true)) => {
            eprintln!("The target executable must be a file: {:?}", target);
            std::process::exit(RET_ENV_ERROR);
        }
        Ok((_, _, false)) => {
            eprintln!("The target executable permissions must include the SUID bit as well as be writable by only the owning user: {:?}", target);
            std::process::exit(RET_PERM_TARGET);
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("Unable to find the owner of the target executable {:?}: {}", target, err);
            std::process::exit(RET_NO_TARGET);
        }
        Err(err) => {
            eprintln!("Unable to find the owner of the target executable {:?}: {}", target, err);
            std::process::exit(RET_ENV_ERROR);
        }
    };
    if euid != tar_uid {
        eprintln!("The the owner of the target executable is not the same as the executable.");
        std::process::exit(RET_OWNER_TARGET);
    }

    let mut command = Command::new(target);
    command.current_dir(cwd)
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .env_clear();
    Env::prepare_command(&mut command, args, exe_uid, gid);

    let r = Env::wait_for(command);
    std::process::exit(r);
}
