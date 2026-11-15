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
    /// Single compressed files.
    Gz,
    Xz,
    Bz2,
    Zst,
    Iso,
    Cab,
    Lzh,
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
            Format::Gz => "GZ",
            Format::Xz => "XZ",
            Format::Bz2 => "BZ2",
            Format::Zst => "ZST",
            Format::Iso => "ISO",
            Format::Cab => "CAB",
            Format::Lzh => "LZH",
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
            Format::Gz => "gz",
            Format::Xz => "xz",
            Format::Bz2 => "bz2",
            Format::Zst => "zst",
            Format::Iso => "iso",
            Format::Cab => "cab",
            Format::Lzh => "lzh",
        }
    }

    /// Formats Zipkoo can write. RAR is proprietary and is open-only.
    pub fn can_create(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZ | Format::TarZst)
    }

    /// Read through `formats::walk` (ISO, CAB, LZH, single compressed files).
    pub fn is_simple(self) -> bool {
        matches!(self, Format::Gz | Format::Xz | Format::Bz2 | Format::Zst | Format::Iso | Format::Cab | Format::Lzh)
    }

    pub fn supports_password(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZ)
    }

    pub fn from_name(name: &str) -> Option<Format> {
        let n = name.to_ascii_lowercase();
        let n = crate::split::strip_part_suffix(&n);
        Some(if n.ends_with(".zip") || n.ends_with(".zipx") {
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
        } else if n.ends_with(".iso") {
            Format::Iso
        } else if n.ends_with(".cab") {
            Format::Cab
        } else if n.ends_with(".lzh") || n.ends_with(".lha") {
            Format::Lzh
        } else if n.ends_with(".gz") {
            Format::Gz
        } else if n.ends_with(".xz") {
            Format::Xz
        } else if n.ends_with(".bz2") {
            Format::Bz2
        } else if n.ends_with(".zst") {
            Format::Zst
        } else {
            return None;
        })
    }

    fn stream_magic(h: &[u8]) -> Option<Format> {
        if h.starts_with(&[0x1f, 0x8b]) {
            Some(Format::Gz)
        } else if h.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
            Some(Format::Xz)
        } else if h.starts_with(b"BZh") {
            Some(Format::Bz2)
        } else if h.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
            Some(Format::Zst)
        } else {
            None
        }
    }

    /// Decompress the first block and look for the "ustar" signature.
    fn sniff_tar(path: &Path, single: Format) -> Format {
        let as_tar = match single {
            Format::Gz => Format::TarGz,
            Format::Xz => Format::TarXz,
            Format::Bz2 => Format::TarBz2,
            _ => Format::TarZst,
        };
        let mut block = [0u8; 512];
        let ok = File::open(path)
            .ok()
            .and_then(|f| crate::formats::decoder(single, std::io::BufReader::new(f)).ok())
            .map(|mut d| {
                let mut got = 0;
                while got < 512 {
                    match d.read(&mut block[got..]) {
                        Ok(0) | Err(_) => break,
                        Ok(k) => got += k,
                    }
                }
                got == 512 && &block[257..262] == b"ustar"
            })
            .unwrap_or(false);
        if ok { as_tar } else { single }
    }

    fn is_iso(path: &Path) -> bool {
        use std::io::{Seek, SeekFrom};
        let mut buf = [0u8; 5];
        File::open(path)
            .and_then(|mut f| {
                f.seek(SeekFrom::Start(0x8001))?;
                f.read_exact(&mut buf)
            })
            .map(|_| &buf == b"CD001")
            .unwrap_or(false)
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
        } else if h.starts_with(b"MSCF") {
            Some(Format::Cab)
        } else if n >= 5 && &h[2..3] == b"-" && (&h[3..5] == b"lh" || &h[3..5] == b"lz") {
            Some(Format::Lzh)
        } else if let Some(single) = Self::stream_magic(h) {
            // A compressed stream: a TAR inside makes it a .tar.*, anything else a single file.
            Some(Self::sniff_tar(path, single))
        } else if Self::is_iso(path) {
            Some(Format::Iso)
        } else {
            None
        };
        let by_name = path.file_name().and_then(|n| n.to_str()).and_then(Format::from_name);
        // Zipkoo's own self-extracting programs carry a ZIP after the stub.
        let sfx = || {
            File::open(path).ok().and_then(|mut f| crate::sfx::locate_payload(&mut f).ok().flatten()).map(|_| Format::Zip)
        };
        // Compressed tars: magic only says gzip/zstd/etc, so trust the name for those.
        by_magic.or_else(sfx).or(by_name)
    }
}
