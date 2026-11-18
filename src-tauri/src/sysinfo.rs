use serde::Serialize;

pub fn memory_gb() -> f64 {
    let mut s = sysinfo::System::new();
    s.refresh_memory();
    s.total_memory() as f64 / 1_073_741_824.0
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info { pub os: String, pub arch: String, pub memory_gb: f64, pub cpus: usize, pub apple_silicon: bool, pub free_disk_gb: f64, pub gpu: bool }

pub fn info() -> Info {
    Info {
        os: sysinfo::System::long_os_version().unwrap_or_else(|| format!("{} {}", sysinfo::System::name().unwrap_or_default(), sysinfo::System::os_version().unwrap_or_default())),
        arch: std::env::consts::ARCH.into(),
        memory_gb: memory_gb(),
        cpus: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        free_disk_gb: free_disk_gb(),
        gpu: crate::engines::gpu_available(),
    }
}

pub fn free_disk_gb() -> f64 {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let target = crate::paths::app_data();
    disks.list().iter().filter(|d| target.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space() as f64 / 1_073_741_824.0).unwrap_or(0.0)
}
