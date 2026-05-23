use std::time::Duration;

use crate::desktop::fs_setup::sync_input_device_nodes;
use crate::desktop::process::run_quiet;

pub fn start_input_device_node_watcher() {
    std::thread::Builder::new()
        .name("input-device-node-watcher".into())
        .spawn(|| {
            if watch_input_device_nodes().is_err() {
                poll_input_device_nodes();
            }
        })
        .ok();
}

fn sync_and_trigger_if_changed() {
    let mut changed = sync_input_device_nodes();
    for delay in [Duration::from_millis(75), Duration::from_millis(250)] {
        std::thread::sleep(delay);
        changed |= sync_input_device_nodes();
    }
    if changed {
        let _ = run_quiet(&["udevadm", "trigger", "--action=add", "-s", "input"]);
    }
}

fn poll_input_device_nodes() {
    loop {
        sync_and_trigger_if_changed();
        std::thread::sleep(Duration::from_secs(5));
    }
}

#[cfg(target_os = "linux")]
fn watch_input_device_nodes() -> std::io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::path::Path;

    sync_and_trigger_if_changed();

    let raw_fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if raw_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    add_watch(fd.as_raw_fd(), Path::new("/sys/class/input"))?;
    add_watch(fd.as_raw_fd(), Path::new("/dev/input"))?;

    let mut buffer = [0u8; 8192];
    loop {
        let read = unsafe {
            libc::read(
                fd.as_raw_fd(),
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            )
        };
        if read < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if read == 0 {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        sync_and_trigger_if_changed();
    }
}

#[cfg(target_os = "linux")]
fn add_watch(fd: std::os::fd::RawFd, path: &std::path::Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mask = libc::IN_ATTRIB
        | libc::IN_CREATE
        | libc::IN_DELETE
        | libc::IN_DELETE_SELF
        | libc::IN_MOVED_FROM
        | libc::IN_MOVED_TO
        | libc::IN_MOVE_SELF;
    let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), mask) };
    if wd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn watch_input_device_nodes() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "input device event watching requires Linux inotify",
    ))
}
