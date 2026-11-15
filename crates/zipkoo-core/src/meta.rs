//! File timestamps. ZIP stores 2-second "DOS" dates in local time (that's how Windows Explorer,
//! macOS and 7-Zip read them), so Zipkoo converts with this computer's UTC offset.
//! If the offset can't be read (some Linux setups), UTC is used for both directions,
//! which still round-trips exactly inside Zipkoo.
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn offset() -> time::UtcOffset {
    static OFF: OnceLock<time::UtcOffset> = OnceLock::new();
    *OFF.get_or_init(|| time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
}

pub fn zip_time(t: SystemTime) -> Option<zip::DateTime> {
    let odt = time::OffsetDateTime::from(t).to_offset(offset());
    zip::DateTime::try_from(time::PrimitiveDateTime::new(odt.date(), odt.time())).ok()
}

pub fn from_zip_time(d: zip::DateTime) -> Option<SystemTime> {
    let p: time::PrimitiveDateTime = d.try_into().ok()?;
    let secs = p.assume_offset(offset()).unix_timestamp();
    (secs >= 0).then(|| UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// Round to the ZIP 2-second step so values compare cleanly.
pub fn zip_round(t: SystemTime) -> Option<SystemTime> {
    zip_time(t).and_then(from_zip_time)
}

pub fn set_mtime(path: &Path, t: SystemTime) {
    let _ = filetime::set_file_mtime(path, filetime::FileTime::from_system_time(t));
}

/// Windows "Mark of the Web": the Zone.Identifier stream that browsers attach to downloads.
/// Copying it to extracted files keeps SmartScreen and Office Protected View working, as
/// Explorer and 7-Zip do. No-op on other systems.
#[cfg(windows)]
pub fn motw_read(archive: &Path) -> Option<Vec<u8>> {
    let mut s = archive.as_os_str().to_owned();
    s.push(":Zone.Identifier");
    let data = std::fs::read(std::path::PathBuf::from(s)).ok()?;
    // Only zones 3 (Internet) and 4 (Restricted) matter.
    let text = String::from_utf8_lossy(&data);
    text.lines()
        .any(|l| matches!(l.trim(), "ZoneId=3" | "ZoneId=4"))
        .then_some(data)
}

#[cfg(not(windows))]
pub fn motw_read(_archive: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
pub fn motw_write(file: &Path, data: &[u8]) {
    let mut s = file.as_os_str().to_owned();
    s.push(":Zone.Identifier");
    let _ = std::fs::write(std::path::PathBuf::from(s), data);
}

#[cfg(not(windows))]
pub fn motw_write(_file: &Path, _data: &[u8]) {}
