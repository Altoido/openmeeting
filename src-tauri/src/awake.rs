//! Keeps the computer from going to sleep while recording or processing.
//! Windows: SetThreadExecutionState on a helper thread. macOS: the built-in `caffeinate` tool.
//! The display may still turn off; only system sleep is prevented. Closing a laptop lid can still sleep it.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static HOLDERS: AtomicUsize = AtomicUsize::new(0);
static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

struct Worker {
    #[cfg(target_os = "macos")] child: std::process::Child,
    #[cfg(windows)] stop: std::sync::mpsc::Sender<()>,
}

/// While a Guard is alive the system stays awake.
pub struct Guard(());
impl Drop for Guard {
    fn drop(&mut self) { if HOLDERS.fetch_sub(1, Ordering::SeqCst) == 1 { stop(); } }
}

pub fn hold(reason: &str) -> Guard {
    if HOLDERS.fetch_add(1, Ordering::SeqCst) == 0 { start(reason); }
    Guard(())
}

pub fn active() -> bool { HOLDERS.load(Ordering::SeqCst) > 0 }

#[allow(unused_variables)]
fn start(reason: &str) {
    let mut w = WORKER.lock().unwrap();
    if w.is_some() { return; }
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id().to_string();
        if let Ok(child) = std::process::Command::new("/usr/bin/caffeinate").args(["-i", "-m", "-w", &pid]).spawn() {
            *w = Some(Worker { child });
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED, ES_AWAYMODE_REQUIRED};
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED);
            let _ = rx.recv();
            SetThreadExecutionState(ES_CONTINUOUS);
        });
        *w = Some(Worker { stop: tx });
    }
    crate::paths::log(&format!("keeping the computer awake: {reason}"));
}

fn stop() {
    let Some(w) = WORKER.lock().unwrap().take() else { return };
    #[cfg(target_os = "macos")]
    { let mut w = w; let _ = w.child.kill(); let _ = w.child.wait(); }
    #[cfg(windows)]
    { let _ = w.stop.send(()); }
    #[cfg(not(any(target_os = "macos", windows)))]
    { let _ = w; }
    crate::paths::log("allowing sleep again");
}

#[cfg(test)]
mod tests {
    #[test]
    fn nested_guards() {
        let a = super::hold("a"); let b = super::hold("b");
        assert!(super::active()); drop(a); assert!(super::active()); drop(b); assert!(!super::active());
    }
}
