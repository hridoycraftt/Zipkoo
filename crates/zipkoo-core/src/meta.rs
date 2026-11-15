//! File timestamps. ZIP stores 2-second local-style dates; Zipkoo writes and reads them as UTC
//! so a round trip (and Smart Update's change detection) is exact to the 2-second step.
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn zip_time(t: SystemTime) -> Option<zip::DateTime> {
    let odt = time::OffsetDateTime::from(t);
    zip::DateTime::try_from(time::PrimitiveDateTime::new(odt.date(), odt.time())).ok()
}

pub fn from_zip_time(d: zip::DateTime) -> Option<SystemTime> {
    let p: time::PrimitiveDateTime = d.try_into().ok()?;
    let secs = p.assume_utc().unix_timestamp();
    (secs >= 0).then(|| UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// Round to the ZIP 2-second step so values compare cleanly.
pub fn zip_round(t: SystemTime) -> Option<SystemTime> {
    zip_time(t).and_then(from_zip_time)
}

pub fn set_mtime(path: &Path, t: SystemTime) {
    let _ = filetime::set_file_mtime(path, filetime::FileTime::from_system_time(t));
}
