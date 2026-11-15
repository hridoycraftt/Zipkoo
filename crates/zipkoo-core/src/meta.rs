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
