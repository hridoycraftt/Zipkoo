//! Zipkoo core: create, inspect, peek and safely extract archives.
//!
//! Create: ZIP (Deflate, AES-256), 7z (LZMA2, AES-256), TAR.ZST.
//! Open: ZIP, 7z, TAR, TAR.GZ, TAR.ZST, TAR.BZ2, TAR.XZ, RAR, plus split parts (.001).

pub mod error;
pub mod extract;
pub mod format;
pub mod hash;
pub mod inspect;
pub mod pack;
pub mod progress;
pub mod safety;
pub mod sfx;
pub mod smart;
pub mod split;

pub use error::{Error, Result};
pub use extract::{extract, suggest_dest, ExtractOptions, ExtractReport};
pub use format::Format;
pub use inspect::{inspect, peek, ArchiveInfo, Entry};
pub use pack::{pack, unique_path, Level, PackOptions, PackReport};
pub use progress::Progress;
