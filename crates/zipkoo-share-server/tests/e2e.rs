use std::fs;
use std::path::Path;
use zipkoo_core::share;
use zipkoo_share_server::{app, Config};

/// Start a server on a free port in the background; returns its base URL and storage dir.
fn server(max_bytes: u64) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config { dir: dir.path().to_path_buf(), max_bytes, max_hours: 168 };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app(cfg).0).await.unwrap();
        });
    });
    (format!("http://{}", rx.recv().unwrap()), dir)
}

fn file(dir: &Path, name: &str, len: usize) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::write(&p, (0..len).map(|i| (i * 31 % 251) as u8).collect::<Vec<_>>()).unwrap();
    p
}

#[test]
fn share_round_trip_and_server_sees_only_ciphertext() {
    let (url, store) = server(1 << 30);
    let t = tempfile::tempdir().unwrap();
    // Not a multiple of the chunk size, plus an empty file.
    for (name, len) in [("Wedding Photos.zip", 3 * share::CHUNK + 777), ("empty.txt", 0)] {
        let src = file(t.path(), name, len);
        let opts = share::ShareOptions { server: &url, expires_hours: 24, max_downloads: None, password: None };
        let s = share::upload(&src, &opts, &mut |_| {}).unwrap();
        assert!(s.url.starts_with(&format!("{url}/s/")) && s.url.contains('#'));
        // The stored blob is not the plaintext and doesn't contain the file name.
        let blob = fs::read(store.path().join(format!("{}.bin", s.id))).unwrap();
        assert!(!blob.windows(7).any(|w| w == b"Wedding"));
        let out = share::download(&s.url, &t.path().join("got"), None, &mut |_| {}).unwrap();
        assert_eq!(out.file_name().unwrap().to_str().unwrap(), name);
        assert_eq!(fs::read(&out).unwrap(), fs::read(&src).unwrap());
    }
}

#[test]
fn password_limit_and_delete() {
    let (url, _store) = server(1 << 30);
    let t = tempfile::tempdir().unwrap();
    let src = file(t.path(), "secret.7z", 200_000);
    let opts = share::ShareOptions { server: &url, expires_hours: 1, max_downloads: Some(2), password: Some("correct horse") };
    let s = share::upload(&src, &opts, &mut |_| {}).unwrap();
    let dest = t.path().join("got");
    assert!(matches!(share::download(&s.url, &dest, None, &mut |_| {}), Err(zipkoo_core::Error::PasswordRequired)));
    assert!(matches!(share::download(&s.url, &dest, Some("wrong"), &mut |_| {}), Err(zipkoo_core::Error::WrongPassword)));
    // Both failed attempts counted as downloads, so the limit (2) is reached.
    assert!(share::download(&s.url, &dest, Some("correct horse"), &mut |_| {}).is_err(), "limit reached");

    let s = share::upload(&src, &share::ShareOptions { max_downloads: Some(1), ..opts.clone() }, &mut |_| {}).unwrap();
    let got = share::download(&s.url, &dest, Some("correct horse"), &mut |_| {}).unwrap();
    assert_eq!(fs::read(got).unwrap(), fs::read(&src).unwrap());
    assert!(share::download(&s.url, &dest, Some("correct horse"), &mut |_| {}).is_err());

    let s = share::upload(&src, &share::ShareOptions { max_downloads: None, ..opts.clone() }, &mut |_| {}).unwrap();
    assert!(share::delete(&url, &s.id, "not-the-token").is_err());
    share::delete(&url, &s.id, &s.delete_token).unwrap();
    assert!(share::download(&s.url, &dest, Some("correct horse"), &mut |_| {}).is_err());
}

#[test]
fn tampering_and_wrong_key_are_caught() {
    let (url, store) = server(1 << 30);
    let t = tempfile::tempdir().unwrap();
    let src = file(t.path(), "a.bin", 2 * share::CHUNK + 5);
    let opts = share::ShareOptions { server: &url, expires_hours: 1, max_downloads: None, password: None };

    let s = share::upload(&src, &opts, &mut |_| {}).unwrap();
    let blob = store.path().join(format!("{}.bin", s.id));
    let mut b = fs::read(&blob).unwrap();
    let mid = b.len() / 2;
    b[mid] ^= 1;
    fs::write(&blob, &b).unwrap();
    let dest = t.path().join("got");
    assert!(share::download(&s.url, &dest, None, &mut |_| {}).is_err());
    assert_eq!(fs::read_dir(&dest).map(|d| d.count()).unwrap_or(0), 0, "no partial file left behind");

    // Truncated after a whole chunk: the 'last chunk' flag catches it.
    let s = share::upload(&src, &opts, &mut |_| {}).unwrap();
    let blob = store.path().join(format!("{}.bin", s.id));
    let b = fs::read(&blob).unwrap();
    fs::write(&blob, &b[..share::HEADER_LEN + 200]).unwrap();
    assert!(share::download(&s.url, &dest, None, &mut |_| {}).is_err());

    // Another link's key.
    let s1 = share::upload(&src, &opts, &mut |_| {}).unwrap();
    let s2 = share::upload(&src, &opts, &mut |_| {}).unwrap();
    let frankenlink = format!("{}#{}", s1.url.split('#').next().unwrap(), s2.url.split('#').nth(1).unwrap());
    assert!(share::download(&frankenlink, &dest, None, &mut |_| {}).is_err());
    assert!(share::parse_link("https://x/s/abc").is_err());
}

#[test]
fn size_limit_is_enforced() {
    let (url, store) = server(100_000);
    let t = tempfile::tempdir().unwrap();
    let src = file(t.path(), "big.bin", 300_000);
    let opts = share::ShareOptions { server: &url, expires_hours: 1, max_downloads: None, password: None };
    assert!(share::upload(&src, &opts, &mut |_| {}).is_err());
    assert_eq!(fs::read_dir(store.path()).unwrap().count(), 0, "rejected uploads leave nothing behind");
}
