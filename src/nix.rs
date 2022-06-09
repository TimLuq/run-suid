use std::{path::{Path, PathBuf}, os::unix::prelude::{MetadataExt, PermissionsExt}, fs::Metadata, process::Command, collections::BTreeSet};

use crate::{env::EnvTrait, RET_GENERIC_ERROR};

pub(crate) struct Nix {}

impl EnvTrait for Nix {
    #[inline]
    unsafe fn geteuid() -> u32 {
        geteuid()
    }
    #[inline]
    unsafe fn getuid() -> u32 {
        getuid()
    }
    #[inline]
    unsafe fn getegid() -> u32 {
        getegid()
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
    fn prepare_command<'a, A: IntoIterator<Item = &'a str>>(command: &mut Command, args: A, uid: u32, gid: u32) {
        prepare_command(command, args, uid, gid)
    }
    #[inline]
    fn wait_for(child: Command) -> i32 {
        wait_for(child)
    }
}

#[link(name = "c")]
extern "C" {
    pub(crate) fn geteuid() -> u32;
    pub(crate) fn getuid() -> u32;
    pub(crate) fn getegid() -> u32;
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
        r.push(format!("{}.run-suid.{}", &file_name[..(pos - 1)], &file_name[..pos]));
    } else {
        r.push(format!("{}.run-suid", file_name));
    }
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

fn prepare_command<'a, A: IntoIterator<Item = &'a str>>(command: &mut Command, args: A, uid: u32, gid: u32) {
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
    std::os::unix::process::CommandExt::uid(command, uid);
    std::os::unix::process::CommandExt::gid(command, gid);
    command.env("PATH", path);
}

fn wait_for(mut child: Command) -> i32 {
    let e = std::os::unix::process::CommandExt::exec(&mut child);
    eprintln!("Unable to execute command: {}", e);
    RET_GENERIC_ERROR
}
