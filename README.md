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
| Secure Share, Smart Update, Self-heal, Watch Folder, Cloud, global search, Batch convert | 🔜 roadmap |

## Layout

```
crates/zipkoo-core   Rust library: pack, inspect, peek, safe extract, split, hash
crates/zipkoo-cli    `zipkoo` command line
crates/zipkoo-sfx    Self-extracting stub (~750 KB); the payload ZIP is appended to it
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

## Release

Push a tag such as `v0.1.0`. GitHub Actions builds `.msi`/`.exe` (Windows) and `.dmg` (Apple Silicon and Intel)
into a draft release. Add the Apple signing secrets listed in `release.yml` to sign and notarize the Mac build;
Windows signing is set up separately (Azure Trusted Signing or an OV certificate).

## Licensing notes

RAR files are opened with the UnRAR library through the `unrar` crate. Its license allows extracting RAR
archives but forbids using the code to create RAR files or to re-create the RAR compression algorithm, so Zipkoo
never writes RAR. Test fixtures in `crates/zipkoo-core/tests/data` come from the `unrar` crate (MIT/Apache-2.0).
