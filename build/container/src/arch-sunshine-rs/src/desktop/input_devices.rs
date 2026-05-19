use std::time::Duration;

use crate::desktop::fs_setup::sync_input_device_nodes;
use crate::desktop::process::run_quiet;

pub fn start_input_device_node_watcher() {
    std::thread::Builder::new()
        .name("input-device-node-watcher".into())
        .spawn(|| loop {
            if sync_input_device_nodes() {
                let _ = run_quiet(&["udevadm", "trigger", "--action=add", "-s", "input"]);
            }
            std::thread::sleep(Duration::from_millis(200));
        })
        .ok();
}
