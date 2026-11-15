use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    Zip,
    SevenZ,
    Tar,
    TarGz,
    TarZst,
    TarBz2,
    TarXz,
    Rar,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::SevenZ => "7Z",
            Format::Tar => "TAR",
            Format::TarGz => "TAR.GZ",
            Format::TarZst => "TAR.ZST",
            Format::TarBz2 => "TAR.BZ2",
            Format::TarXz => "TAR.XZ",
            Format::Rar => "RAR",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarZst => "tar.zst",
            Format::TarBz2 => "tar.bz2",
            Format::TarXz => "tar.xz",
            Format::Rar => "rar",
        }
    }

    /// Formats Zipkoo can write. RAR is proprietary and is open-only.
    pub fn can_create(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZ | Format::TarZst)
    }

    pub fn supports_password(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZ)
    }

    pub fn from_name(name: &str) -> Option<Format> {
        let n = name.to_ascii_lowercase();
        let n = crate::split::strip_part_suffix(&n);
        Some(if n.ends_with(".zip") {
            Format::Zip
        } else if n.ends_with(".7z") {
            Format::SevenZ
        } else if n.ends_with(".tar.zst") || n.ends_with(".tzst") {
            Format::TarZst
        } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
            Format::TarGz
        } else if n.ends_with(".tar.bz2") || n.ends_with(".tbz2") {
            Format::TarBz2
        } else if n.ends_with(".tar.xz") || n.ends_with(".txz") {
            Format::TarXz
        } else if n.ends_with(".tar") {
            Format::Tar
        } else if n.ends_with(".rar") {
            Format::Rar
        } else {
            return None;
        })
    }

    /// Detect by magic bytes first, then by name.
    pub fn detect(path: &Path) -> Option<Format> {
        let mut head = [0u8; 8];
        let n = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
        let h = &head[..n];
        let by_magic = if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") {
            Some(Format::Zip)
        } else if h.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
            Some(Format::SevenZ)
        } else if h.starts_with(b"Rar!\x1a\x07") {
            Some(Format::Rar)
        } else {
            None
        };
        let by_name = path.file_name().and_then(|n| n.to_str()).and_then(Format::from_name);
        // Compressed tars: magic only says gzip/zstd/etc, so trust the name for those.
        by_magic.or(by_name)
    }
}
