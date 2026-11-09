use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};

#[derive(Clone)]
pub struct DriveInfo {
    pub root: String,
    pub total: u64,
    pub free: u64,
    pub kind: &'static str,
}

pub fn list() -> Vec<DriveInfo> {
    let mask = unsafe { GetLogicalDrives() };
    (0u32..26)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", (b'A' + i as u8) as char);
            let w: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            let kind = match unsafe { GetDriveTypeW(w.as_ptr()) } {
                2 => "Removable",
                3 => "Local",
                4 => "Network",
                5 => "CD/DVD",
                6 => "RAM disk",
                _ => return None,
            };
            let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
            let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
            if ok == 0 {
                return None;
            }
            Some(DriveInfo { root, total, free: avail, kind })
        })
        .collect()
}
