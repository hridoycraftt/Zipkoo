//! Secure Share: end-to-end encrypted links.
//!
//! The file is encrypted here with AES-256-GCM before upload. The key lives in the link's `#`
//! fragment, which browsers never send to a server, so the share server only ever stores
//! ciphertext. An optional password is mixed into the key (PBKDF2-SHA256), so a leaked link alone
//! is not enough. The server enforces expiry and a download limit.
//!
//! Format (all little-endian):
//!   header (45 B): magic[8] | chunk_size u32 | flags u8 (bit0 = password) | nonce_prefix[8]
//!                  | salt[16] | pbkdf2_iterations u32 | meta_len u32
//!   meta: AES-GCM(JSON {name,size}), nonce = prefix|FFFFFFFF, AAD = header
//!   chunks: AES-GCM(plain[i]), nonce = prefix|i (u32 BE), AAD = [1 if last chunk else 0]
//! The "last" flag stops anyone from silently cutting the file short.
use crate::error::{Error, Result};
use aes_gcm::aead::{rand_core::RngCore, Aead, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ZKSHR\x01\0\0";
pub const HEADER_LEN: usize = 45;
pub const CHUNK: usize = 1 << 20;
pub const PBKDF2_ITERS: u32 = 600_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub name: String,
    pub size: u64,
}

fn crypto_err(what: &str) -> Error {
    Error::Damaged(what.into())
}

fn derive(link_key: &[u8; 32], password: Option<&str>, salt: &[u8; 16], iters: u32) -> [u8; 32] {
    match password {
        None => *link_key,
        Some(pw) => {
            let mut pk = [0u8; 32];
            pbkdf2::pbkdf2_hmac::<Sha256>(pw.as_bytes(), salt, iters, &mut pk);
            let mut h = Sha256::new();
            h.update(link_key);
            h.update(pk);
            h.finalize().into()
        }
    }
}

fn nonce(prefix: &[u8; 8], counter: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(prefix);
    n[8..].copy_from_slice(&counter.to_be_bytes());
    n
}

fn fill(r: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

/// Encrypt `src` into `dst`. Returns the link key (goes into the URL fragment).
pub fn encrypt(src: &Path, dst: &Path, display_name: &str, password: Option<&str>, on_bytes: &mut dyn FnMut(u64)) -> Result<[u8; 32]> {
    let size = std::fs::metadata(src)?.len();
    let mut link_key = [0u8; 32];
    let mut prefix = [0u8; 8];
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut link_key);
    OsRng.fill_bytes(&mut prefix);
    OsRng.fill_bytes(&mut salt);
    let key = derive(&link_key, password, &salt, PBKDF2_ITERS);
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();

    let meta = serde_json::to_vec(&Meta { name: display_name.to_string(), size }).unwrap();
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&(CHUNK as u32).to_le_bytes());
    header.push(password.is_some() as u8);
    header.extend_from_slice(&prefix);
    header.extend_from_slice(&salt);
    header.extend_from_slice(&PBKDF2_ITERS.to_le_bytes());
    header.extend_from_slice(&((meta.len() + 16) as u32).to_le_bytes());
    debug_assert_eq!(header.len(), HEADER_LEN);
    let meta_ct = cipher
        .encrypt(Nonce::from_slice(&nonce(&prefix, u32::MAX)), Payload { msg: &meta, aad: &header })
        .map_err(|_| crypto_err("encryption failed"))?;

    let mut out = BufWriter::new(File::create(dst)?);
    out.write_all(&header)?;
    out.write_all(&meta_ct)?;
    let mut input = BufReader::new(File::open(src)?);
    let chunks = size.div_ceil(CHUNK as u64).max(1);
    let mut buf = vec![0u8; CHUNK];
    for i in 0..chunks {
        let n = fill(&mut input, &mut buf)?;
        let last = i + 1 == chunks;
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce(&prefix, i as u32)), Payload { msg: &buf[..n], aad: &[last as u8] })
            .map_err(|_| crypto_err("encryption failed"))?;
        out.write_all(&ct)?;
        on_bytes(n as u64);
    }
    out.flush()?;
    Ok(link_key)
}

/// Read the header of an encrypted stream: does it need a password?
pub fn needs_password(header: &[u8]) -> bool {
    header.len() >= 13 && header[12] & 1 == 1
}

/// Decrypt a stream into `dest_dir`. Returns the written file (named from the encrypted metadata).
pub fn decrypt(src: &mut dyn Read, dest_dir: &Path, link_key: &[u8; 32], password: Option<&str>, on_bytes: &mut dyn FnMut(u64)) -> Result<PathBuf> {
    let mut header = [0u8; HEADER_LEN];
    src.read_exact(&mut header).map_err(|_| crypto_err("the shared file is incomplete"))?;
    if &header[..8] != MAGIC {
        return Err(crypto_err("not a Zipkoo share"));
    }
    let chunk = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let has_pw = header[12] & 1 == 1;
    let prefix: [u8; 8] = header[13..21].try_into().unwrap();
    let salt: [u8; 16] = header[21..37].try_into().unwrap();
    let iters = u32::from_le_bytes(header[37..41].try_into().unwrap());
    let meta_len = u32::from_le_bytes(header[41..45].try_into().unwrap()) as usize;
    if chunk == 0 || chunk > 64 << 20 || meta_len > 1 << 16 {
        return Err(crypto_err("the shared file header is invalid"));
    }
    if has_pw && password.is_none() {
        return Err(Error::PasswordRequired);
    }
    let key = derive(link_key, if has_pw { password } else { None }, &salt, iters);
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
    let mut meta_ct = vec![0u8; meta_len];
    src.read_exact(&mut meta_ct).map_err(|_| crypto_err("the shared file is incomplete"))?;
    let meta = cipher
        .decrypt(Nonce::from_slice(&nonce(&prefix, u32::MAX)), Payload { msg: &meta_ct, aad: &header })
        .map_err(|_| if has_pw { Error::WrongPassword } else { crypto_err("this link's key doesn't match the file") })?;
    let meta: Meta = serde_json::from_slice(&meta).map_err(|_| crypto_err("bad metadata"))?;
    // Only a plain file name, never a path.
    let name = Path::new(&meta.name).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "shared-file".into());
    std::fs::create_dir_all(dest_dir)?;
    let out_path = crate::pack::unique_path(&dest_dir.join(name));
    let tmp = out_path.with_extension("zkshare.part");
    let res = (|| -> Result<()> {
        let mut out = BufWriter::new(File::create(&tmp)?);
        let chunks = meta.size.div_ceil(chunk as u64).max(1);
        let mut buf = vec![0u8; chunk + 16];
        for i in 0..chunks {
            let plain_len = if i + 1 == chunks { (meta.size - i * chunk as u64) as usize } else { chunk };
            let ct = &mut buf[..plain_len + 16];
            src.read_exact(ct).map_err(|_| crypto_err("the download stopped early. Try again."))?;
            let last = i + 1 == chunks;
            let pt = cipher
                .decrypt(Nonce::from_slice(&nonce(&prefix, i as u32)), Payload { msg: ct, aad: &[last as u8] })
                .map_err(|_| crypto_err("the shared file was changed or damaged"))?;
            out.write_all(&pt)?;
            on_bytes(pt.len() as u64);
        }
        let mut extra = [0u8; 1];
        if src.read(&mut extra)? != 0 {
            return Err(crypto_err("the shared file has unexpected extra data"));
        }
        out.flush()?;
        Ok(())
    })();
    match res {
        Ok(()) => {
            std::fs::rename(&tmp, &out_path)?;
            Ok(out_path)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// ---------- Links and the share server API ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shared {
    pub url: String,
    pub id: String,
    pub delete_token: String,
    /// Seconds since 1970 (UTC).
    pub expires_at: u64,
    pub max_downloads: Option<u32>,
    pub password: bool,
}

#[derive(Debug, Clone)]
pub struct ShareOptions<'a> {
    pub server: &'a str,
    pub expires_hours: u32,
    pub max_downloads: Option<u32>,
    pub password: Option<&'a str>,
}

pub fn make_link(server: &str, id: &str, link_key: &[u8; 32]) -> String {
    format!("{}/s/{}#{}", server.trim_end_matches('/'), id, B64.encode(link_key))
}

/// Split a share link into (server, id, key).
pub fn parse_link(link: &str) -> Result<(String, String, [u8; 32])> {
    let bad = || Error::UnsupportedFormat("That doesn't look like a Zipkoo share link.".into());
    let (before, frag) = link.trim().split_once('#').ok_or_else(bad)?;
    let (server, id) = before.rsplit_once("/s/").ok_or_else(bad)?;
    let key: [u8; 32] = B64.decode(frag).ok().and_then(|k| k.try_into().ok()).ok_or_else(bad)?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(bad());
    }
    Ok((server.to_string(), id.to_string(), key))
}

fn http_err(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Status(404, _) | ureq::Error::Status(410, _) => {
            Error::EntryNotFound("This link has expired, reached its download limit, or was deleted.".into())
        }
        ureq::Error::Status(413, _) => Error::TooLarge { limit: 0 },
        ureq::Error::Status(code, r) => Error::Damaged(format!("share server said {code}: {}", r.into_string().unwrap_or_default())),
        ureq::Error::Transport(t) => Error::Io(io::Error::other(format!("Can't reach the share server: {t}"))),
    }
}

struct Counting<'a, R> {
    inner: R,
    cb: &'a mut dyn FnMut(u64),
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.map(|c| c.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(false) {
            return Err(io::Error::other("zipkoo: cancelled"));
        }
        let n = self.inner.read(buf)?;
        (self.cb)(n as u64);
        Ok(n)
    }
}

fn cancelled(e: Error) -> Error {
    match e {
        Error::Io(io) if io.to_string().contains("zipkoo: cancelled") => Error::Cancelled,
        Error::Damaged(m) if m.contains("zipkoo: cancelled") => Error::Cancelled,
        other => other,
    }
}

/// Check the password against the share's header without downloading the file.
/// The server's `/head` route does not count as a download.
pub fn check(link: &str, password: Option<&str>) -> Result<Meta> {
    let (server, id, key) = parse_link(link)?;
    let resp = ureq::get(&format!("{server}/api/shares/{id}/head")).call().map_err(http_err)?;
    let mut head = Vec::new();
    resp.into_reader().take(HEADER_LEN as u64 + 65_536 + 16).read_to_end(&mut head)?;
    if head.len() < HEADER_LEN || &head[..8] != MAGIC {
        return Err(crypto_err("not a Zipkoo share"));
    }
    let header: [u8; HEADER_LEN] = head[..HEADER_LEN].try_into().unwrap();
    let has_pw = needs_password(&header);
    if has_pw && password.is_none() {
        return Err(Error::PasswordRequired);
    }
    let prefix: [u8; 8] = header[13..21].try_into().unwrap();
    let salt: [u8; 16] = header[21..37].try_into().unwrap();
    let iters = u32::from_le_bytes(header[37..41].try_into().unwrap());
    let meta_len = u32::from_le_bytes(header[41..45].try_into().unwrap()) as usize;
    let meta_ct = head.get(HEADER_LEN..HEADER_LEN + meta_len).ok_or_else(|| crypto_err("the shared file is incomplete"))?;
    let k = derive(&key, if has_pw { password } else { None }, &salt, iters);
    let cipher = Aes256Gcm::new_from_slice(&k).unwrap();
    let meta = cipher
        .decrypt(Nonce::from_slice(&nonce(&prefix, u32::MAX)), Payload { msg: meta_ct, aad: &header })
        .map_err(|_| if has_pw { Error::WrongPassword } else { crypto_err("this link's key doesn't match the file") })?;
    serde_json::from_slice(&meta).map_err(|_| crypto_err("bad metadata"))
}

/// Encrypt `file` and upload it. `on_bytes` gets encrypted then uploaded byte counts.
pub fn upload(file: &Path, opts: &ShareOptions, on_bytes: &mut dyn FnMut(u64)) -> Result<Shared> {
    upload_cancellable(file, opts, on_bytes, None)
}

pub fn upload_cancellable(file: &Path, opts: &ShareOptions, on_bytes: &mut dyn FnMut(u64), cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<Shared> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let mut tag = [0u8; 8];
    OsRng.fill_bytes(&mut tag);
    let tag: String = tag.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = std::env::temp_dir().join(format!("zipkoo-share-{tag}.bin"));
    let mut enc_cb = |n: u64| on_bytes(n);
    let key = if cancel.map(|c| c.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(false) {
        Err(Error::Cancelled)
    } else {
        encrypt(file, &tmp, &name, opts.password, &mut enc_cb)
    };
    let res = (|| -> Result<Shared> {
        let key = key?;
        let len = std::fs::metadata(&tmp)?.len();
        let body = Counting { inner: BufReader::new(File::open(&tmp)?), cb: on_bytes, cancel };
        let mut req = ureq::post(&format!("{}/api/shares", opts.server.trim_end_matches('/')))
            .set("Content-Type", "application/octet-stream")
            .set("Content-Length", &len.to_string())
            .set("X-Expires-Hours", &opts.expires_hours.to_string());
        if let Some(m) = opts.max_downloads {
            req = req.set("X-Max-Downloads", &m.to_string());
        }
        let resp: serde_json::Value = serde_json::from_reader(req.send(body).map_err(http_err)?.into_reader()).map_err(|_| crypto_err("bad server reply"))?;
        let id = resp["id"].as_str().ok_or_else(|| crypto_err("bad server reply"))?.to_string();
        Ok(Shared {
            url: make_link(opts.server, &id, &key),
            delete_token: resp["deleteToken"].as_str().unwrap_or_default().to_string(),
            expires_at: resp["expiresAt"].as_u64().unwrap_or(0),
            max_downloads: opts.max_downloads,
            password: opts.password.is_some(),
            id,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    res.map_err(cancelled)
}

/// Download and decrypt a share link into `dest_dir`. The password is checked first,
/// so a missing or wrong password never uses up one of the link's downloads.
pub fn download(link: &str, dest_dir: &Path, password: Option<&str>, on_bytes: &mut dyn FnMut(u64)) -> Result<PathBuf> {
    download_cancellable(link, dest_dir, password, on_bytes, None)
}

pub fn download_cancellable(link: &str, dest_dir: &Path, password: Option<&str>, on_bytes: &mut dyn FnMut(u64), cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<PathBuf> {
    check(link, password)?;
    let (server, id, key) = parse_link(link)?;
    let resp = ureq::get(&format!("{server}/api/shares/{id}/blob")).call().map_err(http_err)?;
    let mut noop = |_n: u64| {};
    let mut reader = BufReader::new(Counting { inner: resp.into_reader(), cb: &mut noop, cancel });
    decrypt(&mut reader, dest_dir, &key, password, on_bytes).map_err(cancelled)
}

pub fn delete(server: &str, id: &str, token: &str) -> Result<()> {
    ureq::delete(&format!("{}/api/shares/{id}", server.trim_end_matches('/'))).set("X-Delete-Token", token).call().map_err(http_err)?;
    Ok(())
}
