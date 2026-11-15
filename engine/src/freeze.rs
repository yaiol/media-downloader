//! Freeze and thaw a program and everything it started (a pause). yt-dlp's standalone build is a parent that runs a child,
//! so freezing only the pid we hold would leave the child downloading.

/// Every process under `root` (children, their children…), from a list of `(pid, parent pid)`. `root` is not in the answer.
pub fn descendants(all: &[(u32, u32)], root: u32) -> Vec<u32> {
    let mut found: Vec<u32> = Vec::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for &(pid, ppid) in all {
            if ppid == parent && pid != root && !found.contains(&pid) {
                found.push(pid);
                frontier.push(pid);
            }
        }
    }
    found
}

/// Freeze the process `pid` and its descendants: they stop using the processor and the network until `thaw`.
pub fn freeze(pid: u32) {
    for p in tree(pid) {
        imp::suspend(p);
    }
}

/// Let a frozen process and its descendants run again.
pub fn thaw(pid: u32) {
    for p in tree(pid) {
        imp::resume(p);
    }
}

fn tree(pid: u32) -> Vec<u32> {
    let mut all = descendants(&imp::processes(), pid);
    all.insert(0, pid);
    all
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SUSPEND_RESUME};

    // Not in the documented API set, but the same two calls every process tool uses (Process Explorer's Suspend/Resume).
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(process: HANDLE) -> i32;
        fn NtResumeProcess(process: HANDLE) -> i32;
    }

    pub fn processes() -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                out.push((entry.th32ProcessID, entry.th32ParentProcessID));
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        out
    }

    fn with_process(pid: u32, call: unsafe extern "system" fn(HANDLE) -> i32) {
        unsafe {
            let h = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
            if !h.is_null() {
                call(h);
                CloseHandle(h);
            }
        }
    }

    pub fn suspend(pid: u32) {
        with_process(pid, NtSuspendProcess);
    }

    pub fn resume(pid: u32) {
        with_process(pid, NtResumeProcess);
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::{Command, Stdio};

    pub fn processes() -> Vec<(u32, u32)> {
        let out = Command::new("ps").args(["-A", "-o", "pid=,ppid="]).stderr(Stdio::null()).output();
        let Ok(out) = out else { return Vec::new() };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
            })
            .collect()
    }

    fn signal(pid: u32, name: &str) {
        let _ = Command::new("kill").args([name, &pid.to_string()]).stderr(Stdio::null()).status();
    }

    pub fn suspend(pid: u32) {
        signal(pid, "-STOP");
    }

    pub fn resume(pid: u32) {
        signal(pid, "-CONT");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_tree_under_a_process_is_found_and_nothing_else() {
        // 10 -> 11 -> 12, 10 -> 13; 20 -> 21 is another family.
        let all = [(10, 1), (11, 10), (12, 11), (13, 10), (20, 1), (21, 20)];
        let mut got = descendants(&all, 10);
        got.sort();
        assert_eq!(got, vec![11, 12, 13]);
        assert!(descendants(&all, 21).is_empty());
    }

    #[test]
    fn a_loop_in_the_list_does_not_hang() {
        let all = [(1, 2), (2, 1)];
        let mut got = descendants(&all, 1);
        got.sort();
        assert_eq!(got, vec![2]);
    }

    #[test]
    fn this_process_lists_itself_with_a_parent() {
        let me = std::process::id();
        assert!(imp::processes().iter().any(|&(p, pp)| p == me && pp != 0));
    }

    /// A real tree: cmd runs ping (one line a second). Frozen, no line comes; thawed, they come again.
    #[cfg(windows)]
    #[test]
    fn a_frozen_tree_goes_quiet_and_a_thawed_one_talks_again() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        use std::sync::mpsc;
        use std::time::Duration;
        let mut child = Command::new("cmd").args(["/c", "ping", "-n", "30", "127.0.0.1"]).stdout(Stdio::piped()).stdin(Stdio::null()).spawn().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // Bytes, not text: the machine's code page is not UTF-8.
            let mut r = BufReader::new(out);
            let mut buf = Vec::new();
            while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
                if tx.send(buf.len()).is_err() {
                    break;
                }
                buf.clear();
            }
        });
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok(), "it talks before the freeze");
        freeze(child.id());
        while rx.recv_timeout(Duration::from_millis(1500)).is_ok() {} // what was already on its way
        assert!(rx.recv_timeout(Duration::from_millis(3500)).is_err(), "frozen, it says nothing");
        thaw(child.id());
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok(), "thawed, it talks again");
        let _ = child.kill();
        let _ = child.wait();
    }
}
