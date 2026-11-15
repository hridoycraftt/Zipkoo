# Zipkoo

**Zip it. Ship it.** A friendly archiver for Windows and macOS.

## What works today (v0.1)

| Area | Status |
|---|---|
| Create ZIP (Deflate), 7z (LZMA2), TAR.ZST (multithreaded) | ✅ |
| Open ZIP, 7z, TAR, TAR.GZ, TAR.ZST, TAR.BZ2, TAR.XZ, RAR | ✅ |
| AES-256 passwords (ZIP, 7z) and hidden file names (7z) | ✅ |
| Split into parts (`.001`, `.002`…) and open split archives | ✅ |
| Smart Compress: stores JPG/MP4/ZIP… as they are | ✅ |
| Safe Extract: blocks `../` paths, zip bombs, links; flags and can skip programs | ✅ |
| Peek: preview images, text and PDF without extracting | ✅ |
| Desktop app (Tauri): drag and drop, progress with cancel, English + বাংলা | ✅ |
| `zipkoo` command line | ✅ |
| Right-click menu: Explorer (registry, per user) and Finder (Quick Actions) | ✅ |
| Self-extracting `.exe` (Windows), optional AES-256 password | ✅ |
| From 0-byte files to files over 4 GB (ZIP64), 100,000+ entries, free-space check | ✅ |
| Secure Share: end-to-end encrypted links with expiry, download limit, optional password | ✅ (needs a deployed share server) |
| Smart Update (ZIP): only new/changed files, version history inside the archive | ✅ |
| Self-heal: Reed-Solomon repair data (`.zkrec`), fixes damage and cut-off downloads | ✅ |
| Watch Folder: scheduled snapshots when a folder changes, keep newest N | ✅ |
| Cloud: Google Drive, Dropbox, OneDrive, iCloud folders on this computer | ✅ (desktop-app folders, no direct API) |
| Search inside every archive, duplicate finder, photo shrinking | ✅ |
| Batch Convert (incl. RAR → ZIP/7z/TAR.ZST) and a full CLI | ✅ |

## Layout

```
crates/zipkoo-core   Rust library: pack, inspect, peek, safe extract, split, hash
crates/zipkoo-cli    `zipkoo` command line
crates/zipkoo-sfx    Self-extracting stub (~750 KB); the payload ZIP is appended to it
crates/zipkoo-share-server  Stores encrypted shares; serves the in-browser download page
app/                 Desktop app: React + Vite UI, Tauri 2 shell in app/src-tauri
.github/workflows    CI tests on Windows/macOS/Linux; tagged releases build installers
```

## Develop

Needs Rust (stable), Node 22, and on Linux the Tauri system packages (`libwebkit2gtk-4.1-dev` etc.).

```bash
cargo test -p zipkoo-core -p zipkoo-cli -p zipkoo-sfx   # tests
cargo test -p zipkoo-core --release -- --ignored huge     # >4 GB test, writes ~9 GB temp data
cd app && npm install && npm run tauri dev # run the desktop app
cargo run -p zipkoo-cli -- pack ./Photos -f 7z --lock
```

## Right-click menu

- **Windows:** the NSIS installer runs `Zipkoo.exe --register-shell` (and `--unregister-shell` on uninstall).
  The switch in the app's sidebar does the same, which also covers MSI installs. Entries live under
  `HKCU\Software\Classes`, so no admin rights are needed. On Windows 11 they appear under "Show more options".
- **macOS:** the sidebar switch writes Quick Actions to `~/Library/Services`
  (Finder › right-click › Quick Actions).
- Explorer starts one process per selected file; the app merges clicks that arrive within 400 ms
  into one job.

## Share server

Links are end-to-end encrypted: files are encrypted in the app (AES-256-GCM), the key travels in the
link's `#` fragment, and the server only stores ciphertext. Run the server anywhere with a disk and put it
behind HTTPS:

```bash
cargo build --release -p zipkoo-share-server
ZIPKOO_SHARE_DIR=/var/lib/zipkoo ZIPKOO_SHARE_ADDR=0.0.0.0:8080 ./target/release/zipkoo-share-server
```

Optional: `ZIPKOO_SHARE_MAX_BYTES` (default 20 GB), `ZIPKOO_SHARE_MAX_HOURS` (default 168).
Then enter its address in the app under Shared links.

## Release

Push a tag such as `v0.1.0`. GitHub Actions builds `.msi`/`.exe` (Windows) and `.dmg` (Apple Silicon and Intel)
into a draft release. Add the Apple signing secrets listed in `release.yml` to sign and notarize the Mac build;
Windows signing is set up separately (Azure Trusted Signing or an OV certificate).

## Licensing notes

RAR files are opened with the UnRAR library through the `unrar` crate. Its license allows extracting RAR
archives but forbids using the code to create RAR files or to re-create the RAR compression algorithm, so Zipkoo
never writes RAR. Test fixtures in `crates/zipkoo-core/tests/data` come from the `unrar` crate (MIT/Apache-2.0).
