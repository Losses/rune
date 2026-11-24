//! Thumbnailer programs parse whatever files a folder holds, so they run
//! confined: in a bubblewrap container where bwrap works, otherwise under a
//! Landlock ruleset. A system with neither runs no thumbnailer at all.
//!
//! Inside bubblewrap the program sees the file system read-only, without
//! the account's home (only the file it reads is put back) or the session's
//! sockets, without network, and can write only its output folder. Under
//! Landlock it can read and run anything but write only its output folder,
//! and on kernels with network rules it cannot open TCP connections.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

/// How a thumbnailer is confined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Sandbox {
    Bubblewrap(PathBuf),
    /// The Landlock ABI version the kernel offers.
    Landlock(libc::c_long),
}

impl Sandbox {
    /// The strongest confinement this system offers, if any.
    pub(super) fn detect() -> Option<Self> {
        if let Some(bwrap) = find_program(OsStr::new("bwrap"))
            && bubblewrap_works(&bwrap)
        {
            return Some(Self::Bubblewrap(bwrap));
        }
        landlock_abi().map(Self::Landlock)
    }

    /// Run `program` with `args`, confined, reading `input` and writing only
    /// into `output`. True when it exited successfully within `deadline`; a
    /// program still running then is killed.
    pub(super) fn run(
        &self,
        program: &Path,
        args: &[OsString],
        input: &Path,
        output: &Path,
        deadline: Duration,
    ) -> io::Result<bool> {
        let mut command = match self {
            Self::Bubblewrap(bwrap) => bubblewrap(bwrap, program, args, input, output),
            Self::Landlock(_) => {
                let mut command = Command::new(program);
                command.args(args);
                command
            }
        };
        command
            .env_clear()
            .env("TMPDIR", output)
            .current_dir(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        for key in ["PATH", "LANG", "LC_ALL", "XDG_DATA_DIRS"] {
            if let Some(value) = env::var_os(key) {
                command.env(key, value);
            }
        }
        // Kept open until the child has started; the kernel closes it on exec.
        let mut ruleset = None;
        if let Self::Landlock(abi) = self {
            // Under Landlock the real home is read-only; programs that keep
            // caches there get the output folder instead.
            command.env("HOME", output).env("XDG_CACHE_HOME", output);
            let fd = ruleset.insert(landlock_ruleset(*abi, output)?).as_raw_fd();
            // SAFETY: the closure makes only async-signal-safe system calls
            // and touches no memory shared with the parent.
            unsafe {
                command.pre_exec(move || restrict_self(fd));
            }
        }
        let mut child = command.spawn()?;
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status.success());
            }
            if started.elapsed() > deadline {
                // The program leads its own process group; take all of it.
                // SAFETY: kill with a negative id signals that group only.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

fn bubblewrap(
    bwrap: &Path,
    program: &Path,
    args: &[OsString],
    input: &Path,
    output: &Path,
) -> Command {
    // bwrap cannot mount onto a link, and registered programs are often
    // links (NixOS profiles, alternatives), so every path is bound by what
    // it resolves to.
    let real = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (program, input, output) = (real(program), real(input), real(output));
    let (program, input, output) = (program.as_path(), input.as_path(), output.as_path());
    let mut command = Command::new(bwrap);
    command.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]);
    command.args(["--tmpfs", "/tmp"]);
    // Session sockets: D-Bus, Wayland, the audio server.
    if Path::new("/run/user").is_dir() {
        command.args(["--tmpfs", "/run/user"]);
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from)
        && home.is_dir()
        && home != Path::new("/")
    {
        command.arg("--tmpfs").arg(&home);
    }
    // What the hidden folders held that the program needs: itself, the file
    // it reads and the folder it writes.
    for path in [program, input] {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    command.arg("--bind").arg(output).arg(output);
    command.args(["--unshare-all", "--die-with-parent", "--new-session", "--"]);
    command.arg(program).args(args);
    command
}

/// Whether bwrap can build a container here. Some systems ship it but forbid
/// the user namespaces it needs.
fn bubblewrap_works(bwrap: &Path) -> bool {
    Command::new(bwrap)
        .args([
            "--ro-bind",
            "/",
            "/",
            "--unshare-all",
            "--die-with-parent",
            "--",
        ])
        .arg("true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `name` in the folders of `PATH`.
pub(super) fn find_program(name: &OsStr) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return is_executable(candidate).then(|| candidate.to_path_buf());
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

// The Landlock interface from <linux/landlock.h>.
const CREATE_RULESET_VERSION: libc::c_uint = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_TRUNCATE: u64 = 1 << 14;
const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// The Landlock ABI version, or `None` where the kernel has none or it is
/// turned off.
pub(super) fn landlock_abi() -> Option<libc::c_long> {
    // SAFETY: a null attribute with the version flag only queries.
    let version = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    (version > 0).then_some(version)
}

/// Every file system right this ABI knows, so none is left unrestricted.
fn handled_file_rights(abi: libc::c_long) -> u64 {
    match abi {
        1 => (1 << 13) - 1,
        2 => (1 << 14) - 1,
        3 | 4 => (1 << 15) - 1,
        _ => (1 << 16) - 1,
    }
}

/// A ruleset that lets a program read and run anything, write only inside
/// `writable` and `/dev/null`, and, where the ABI has network rules, open no
/// TCP connection.
fn landlock_ruleset(abi: libc::c_long, writable: &Path) -> io::Result<OwnedFd> {
    let network = abi >= 4;
    let attr = RulesetAttr {
        handled_access_fs: handled_file_rights(abi),
        handled_access_net: if network {
            NET_BIND_TCP | NET_CONNECT_TCP
        } else {
            0
        },
    };
    // ABIs before network rules know only the first field.
    let size = if network {
        size_of::<RulesetAttr>()
    } else {
        size_of::<u64>()
    };
    // SAFETY: `attr` is valid for `size` bytes.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr as *const RulesetAttr,
            size,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a new descriptor this function owns.
    let ruleset = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
    let truncate = if abi >= 3 { FS_TRUNCATE } else { 0 };
    let read = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;
    let write =
        FS_WRITE_FILE | FS_REMOVE_FILE | FS_REMOVE_DIR | FS_MAKE_REG | FS_MAKE_DIR | truncate;
    add_rule(&ruleset, Path::new("/"), read)?;
    add_rule(&ruleset, writable, read | write)?;
    add_rule(
        &ruleset,
        Path::new("/dev/null"),
        FS_READ_FILE | FS_WRITE_FILE | truncate,
    )?;
    Ok(ruleset)
}

fn add_rule(ruleset: &OwnedFd, path: &Path, allowed_access: u64) -> io::Result<()> {
    let parent: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(path)?;
    let attr = PathBeneathAttr {
        allowed_access,
        parent_fd: parent.as_raw_fd(),
    };
    // SAFETY: both descriptors are open and `attr` outlives the call.
    let result = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            RULE_PATH_BENEATH,
            &attr as *const PathBeneathAttr,
            0,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Confine the calling process with `ruleset`. Runs in the child between
/// fork and exec, so it makes system calls only.
fn restrict_self(ruleset: RawFd) -> io::Result<()> {
    // SAFETY: plain system calls with constant arguments and an open fd.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    /// A thumbnailer that copies its input to its output, and also tries to
    /// write outside its output folder.
    struct Fixture {
        dir: tempfile::TempDir,
        script: PathBuf,
        input: PathBuf,
        escape: PathBuf,
    }

    fn fixture(body: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("cover.png");
        fs::write(&input, b"pixels").unwrap();
        let escape = dir.path().join("escaped");
        let script = dir.path().join("thumbnailer");
        let body = body.replace("{escape}", &escape.to_string_lossy());
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        Fixture {
            dir,
            script,
            input,
            escape,
        }
    }

    fn confines_writes_to_the_output(sandbox: Sandbox) {
        let fixture = fixture("echo out > '{escape}'\ncp \"$1\" \"$2\"");
        let output = tempfile::tempdir().unwrap();
        let produced = output.path().join("thumbnail.png");
        let args = [fixture.input.clone().into(), produced.clone().into()];
        let succeeded = sandbox
            .run(
                &fixture.script,
                &args,
                &fixture.input,
                output.path(),
                Duration::from_secs(20),
            )
            .unwrap();
        assert!(succeeded, "{sandbox:?}");
        assert_eq!(fs::read(&produced).unwrap(), b"pixels", "{sandbox:?}");
        assert!(!fixture.escape.exists(), "{sandbox:?} let a write escape");
        drop(fixture.dir);
    }

    #[test]
    fn bubblewrap_keeps_writes_inside_the_output_folder() {
        let Some(bwrap) = find_program(OsStr::new("bwrap")).filter(|bwrap| bubblewrap_works(bwrap))
        else {
            eprintln!("bwrap cannot run here; skipped");
            return;
        };
        confines_writes_to_the_output(Sandbox::Bubblewrap(bwrap));
    }

    /// Registered programs are often links on the read-only root, such as a
    /// NixOS profile's `bin`, which bwrap cannot mount onto.
    #[test]
    fn bubblewrap_runs_a_program_reached_through_a_link() {
        let Some(bwrap) = find_program(OsStr::new("bwrap")).filter(|bwrap| bubblewrap_works(bwrap))
        else {
            eprintln!("bwrap cannot run here; skipped");
            return;
        };
        let Ok(visible) = tempfile::tempdir_in("/var/tmp") else {
            eprintln!("/var/tmp is not writable here; skipped");
            return;
        };
        let fixture = fixture("cp \"$1\" \"$2\"");
        let link = visible.path().join("thumbnailer");
        symlink(&fixture.script, &link).unwrap();
        let output = tempfile::tempdir().unwrap();
        let produced = output.path().join("thumbnail.png");
        let args = [fixture.input.clone().into(), produced.clone().into()];
        let succeeded = Sandbox::Bubblewrap(bwrap)
            .run(
                &link,
                &args,
                &fixture.input,
                output.path(),
                Duration::from_secs(20),
            )
            .unwrap();
        assert!(succeeded);
        assert_eq!(fs::read(&produced).unwrap(), b"pixels");
    }

    #[test]
    fn landlock_keeps_writes_inside_the_output_folder() {
        let Some(abi) = landlock_abi() else {
            eprintln!("Landlock is off here; skipped");
            return;
        };
        confines_writes_to_the_output(Sandbox::Landlock(abi));
    }

    #[test]
    fn a_program_past_its_deadline_is_killed() {
        let Some(sandbox) = Sandbox::detect() else {
            eprintln!("no sandbox here; skipped");
            return;
        };
        let fixture = fixture("sleep 30");
        let output = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let succeeded = sandbox
            .run(
                &fixture.script,
                &[],
                &fixture.input,
                output.path(),
                Duration::from_millis(300),
            )
            .unwrap();
        assert!(!succeeded);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
