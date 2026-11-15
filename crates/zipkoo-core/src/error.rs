use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("This archive is locked. Enter its password.")]
    PasswordRequired,
    #[error("Wrong password. Check Caps Lock and try again.")]
    WrongPassword,
    #[error("Zipkoo can't open this kind of file yet: {0}")]
    UnsupportedFormat(String),
    #[error("{0} archives can't be created, only opened.")]
    CreateNotSupported(&'static str),
    #[error("{0} archives can't be locked with a password. Use ZIP or 7z.")]
    EncryptionNotSupported(&'static str),
    #[error("Blocked an unsafe path inside the archive: {0}")]
    UnsafePath(String),
    #[error("Stopped: this archive would unpack to more than {limit} bytes. It may be a zip bomb.")]
    TooLarge { limit: u64 },
    #[error("Not found inside the archive: {0}")]
    EntryNotFound(String),
    #[error("Nothing to pack. Add at least one file or folder.")]
    NoInputs,
    #[error("Missing part: {0}")]
    MissingPart(PathBuf),
    #[error("Archive is damaged: {0}")]
    Damaged(String),
    #[error("Not enough free space: this needs {need} bytes, but only {free} bytes are free.")]
    NoSpace { need: u64, free: u64 },
    #[error("Cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        use zip::result::ZipError as Z;
        match e {
            Z::Io(io) if io.to_string().contains("zipkoo: cancelled") => Error::Cancelled,
            Z::Io(io) => Error::Io(io),
            Z::InvalidPassword => Error::WrongPassword,
            Z::UnsupportedArchive(m) if m.contains("assword") => Error::PasswordRequired,
            other => Error::Damaged(other.to_string()),
        }
    }
}

impl From<sevenz_rust2::Error> for Error {
    fn from(e: sevenz_rust2::Error) -> Self {
        use sevenz_rust2::Error as S;
        match e {
            S::PasswordRequired => Error::PasswordRequired,
            S::MaybeBadPassword(_) => Error::WrongPassword,
            S::Io(io, _) if io.to_string().contains("zipkoo: cancelled") => Error::Cancelled,
            S::Io(io, _) => Error::Io(io),
            S::FileOpen(io, _) => Error::Io(io),
            other => Error::Damaged(other.to_string()),
        }
    }
}

impl From<unrar::error::UnrarError> for Error {
    fn from(e: unrar::error::UnrarError) -> Self {
        use unrar::error::Code;
        match e.code {
            Code::MissingPassword => Error::PasswordRequired,
            Code::BadPassword => Error::WrongPassword,
            Code::BadArchive | Code::BadData | Code::UnknownFormat => Error::Damaged(e.to_string()),
            _ => Error::Damaged(e.to_string()),
        }
    }
}
