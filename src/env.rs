use std::{path::{Path, PathBuf}, fs::Metadata, process::Command};


pub(crate) trait EnvTrait {
    /// Gets the effective user id, might be different from the real user id if the SUID bit is set.
    unsafe fn geteuid() -> u32;
    /// Gets the real user id.
    unsafe fn getuid() -> u32;
    /// Gets the effective group id.
    unsafe fn getegid() -> u32;
    /// Get the owner of the file and the file's [Metadata].
    fn file_owner(path: &Path) -> Result<(u32, Metadata, bool), std::io::Error>;
    /// Compute the location for the target executable.
    fn sibling_target(parent: &Path, file_name: &str) -> PathBuf;

    fn prepare_command<'a, A: IntoIterator<Item = &'a str>>(command: &mut Command, args: A, uid: u32, gid: u32);
    fn wait_for(child: Command) -> i32;
}


