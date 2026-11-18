//! On-disk locations. Everything lives in the per-user app data folder:
//! macOS: ~/Library/Application Support/OpenMeeting Notes
//! Windows: %APPDATA%\OpenMeeting Notes
use std::path::PathBuf;

pub const APP_DIR: &str = "OpenMeeting Notes";

fn ensure(p: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn app_data() -> PathBuf {
    if let Ok(dir) = std::env::var("OMN_DATA_DIR") {
        return ensure(PathBuf::from(dir));
    }
    ensure(dirs::data_dir().unwrap_or_else(std::env::temp_dir).join(APP_DIR))
}
pub fn library() -> PathBuf { ensure(app_data().join("Library")) }
pub fn database() -> PathBuf { library().join("meetings.sqlite") }
pub fn recordings() -> PathBuf { ensure(library().join("Recordings")) }
pub fn in_progress() -> PathBuf { ensure(library().join("In Progress")) }
pub fn cache() -> PathBuf { ensure(library().join("Cache")) }
pub fn meeting_folder(id: &str) -> PathBuf { ensure(recordings().join(id)) }
pub fn models() -> PathBuf { ensure(app_data().join("Models")) }
pub fn model_dir(kind: &str) -> PathBuf { ensure(models().join(kind)) }
pub fn logs() -> PathBuf { ensure(app_data().join("Logs")) }

pub fn log(msg: &str) {
    use std::io::Write;
    let line = format!("{} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    eprint!("{line}");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(logs().join("app.log")) {
        let _ = f.write_all(line.as_bytes());
    }
}
