use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use zipkoo_core::*;

fn sample(dir: &Path) -> PathBuf {
    let root = dir.join("Photos");
    fs::create_dir_all(root.join("raw/empty")).unwrap();
    fs::write(root.join("notes.txt"), "hello zipkoo\n".repeat(5000)).unwrap();
    fs::write(root.join("raw/IMG_0412.jpg"), (0..200_000u32).map(|i| (i.wrapping_mul(2654435761u32) >> 24) as u8).collect::<Vec<_>>()).unwrap();
    fs::write(root.join("বাংলা নাম.txt"), "আপনার ফাইল").unwrap();
    root
}

fn same_tree(a: &Path, b: &Path) {
    for e in walkdir::WalkDir::new(a) {
        let e = e.unwrap();
        let rel = e.path().strip_prefix(a).unwrap();
        let other = b.join(rel);
        assert!(other.exists(), "missing {}", other.display());
        if e.file_type().is_file() {
            assert_eq!(fs::read(e.path()).unwrap(), fs::read(&other).unwrap(), "differs {}", rel.display());
        }
    }
}

fn roundtrip(format: Format, password: Option<&str>) {
    let t = tempfile::tempdir().unwrap();
    let src = sample(t.path());
    let out = t.path().join(format!("out.{}", format.extension()));
    let opts = PackOptions { format, password: password.map(String::from), ..Default::default() };
    let rep = pack(&[src.clone()], &out, &opts, &mut |_| {}, None).unwrap();
    assert_eq!(rep.files, 3);
    let expect_stored = if format == Format::TarZst { 0 } else { 1 };
    assert_eq!(rep.stored_as_is, expect_stored, "smart compress should store the jpg");
    assert!(rep.output_bytes < rep.input_bytes);

    let info = inspect(&out, password).unwrap();
    assert_eq!(info.format, format);
    assert_eq!(info.file_count, 3);
    assert_eq!(info.encrypted, password.is_some() && format != Format::SevenZ || info.encrypted);

    let peeked = peek(&out, "Photos/notes.txt", password, 1 << 20).unwrap();
    assert!(peeked.starts_with(b"hello zipkoo"));

    let dest = t.path().join("x");
    let eopts = ExtractOptions { password, ..Default::default() };
    let r = extract(&out, &dest, &eopts, &mut |_| {}, None).unwrap();
    assert_eq!(r.files, 3);
    same_tree(&src, &dest.join("Photos"));
}

#[test] fn zip_plain() { roundtrip(Format::Zip, None) }
#[test] fn zip_aes() { roundtrip(Format::Zip, Some("s3cret")) }
#[test] fn sevenz_plain() { roundtrip(Format::SevenZ, None) }
#[test] fn sevenz_aes() { roundtrip(Format::SevenZ, Some("s3cret")) }
#[test] fn tar_zst() { roundtrip(Format::TarZst, None) }

#[test]
fn wrong_and_missing_password() {
    for format in [Format::Zip, Format::SevenZ] {
        let t = tempfile::tempdir().unwrap();
        let src = sample(t.path());
        let out = t.path().join(format!("a.{}", format.extension()));
        let opts = PackOptions { format, password: Some("right".into()), ..Default::default() };
        pack(&[src], &out, &opts, &mut |_| {}, None).unwrap();
        let bad = ExtractOptions { password: Some("wrong"), ..Default::default() };
        let e = extract(&out, &t.path().join("x"), &bad, &mut |_| {}, None).unwrap_err();
        assert!(matches!(e, Error::WrongPassword), "{format:?}: {e:?}");
        let none = ExtractOptions::default();
        let e = extract(&out, &t.path().join("y"), &none, &mut |_| {}, None).unwrap_err();
        assert!(matches!(e, Error::PasswordRequired | Error::WrongPassword), "{format:?}: {e:?}");
    }
}

#[test]
fn password_rejected_for_tar() {
    let t = tempfile::tempdir().unwrap();
    let src = sample(t.path());
    let opts = PackOptions { format: Format::TarZst, password: Some("x".into()), ..Default::default() };
    assert!(matches!(pack(&[src], &t.path().join("a.tar.zst"), &opts, &mut |_| {}, None), Err(Error::EncryptionNotSupported(_))));
}

#[test]
fn split_and_join() {
    let t = tempfile::tempdir().unwrap();
    let src = sample(t.path());
    let out = t.path().join("big.7z");
    let opts = PackOptions { format: Format::SevenZ, split_size: Some(50_000), ..Default::default() };
    let rep = pack(&[src.clone()], &out, &opts, &mut |_| {}, None).unwrap();
    assert!(rep.outputs.len() >= 3, "{:?}", rep.outputs);
    assert!(!out.exists());
    let dest = t.path().join("x");
    extract(&rep.outputs[0], &dest, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    same_tree(&src, &dest.join("Photos"));
}

fn evil_zip(path: &Path, names: &[&str], body: &[u8]) {
    let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for n in names {
        z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body).unwrap();
    }
    z.finish().unwrap();
}

#[test]
fn blocks_zip_slip() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("evil.zip");
    evil_zip(&p, &["../escape.txt", "ok.txt"], b"x");
    let dest = t.path().join("out");
    let r = extract(&p, &dest, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!(r.files, 1);
    assert_eq!(r.skipped[0].1, "unsafe path");
    assert!(!t.path().join("escape.txt").exists());
}

#[test]
fn honest_highly_compressible_archive_opens() {
    // 16 MB of zeros packs to a few KB: a huge ratio, but the sizes are honest and fit on disk.
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("zeros.zip");
    evil_zip(&p, &["a.bin", "b.bin"], &vec![0u8; 8 << 20]);
    let opts = ExtractOptions { max_ratio: 10, floor: 0, ..Default::default() };
    let r = extract(&p, &t.path().join("out"), &opts, &mut |_| {}, None).unwrap();
    assert_eq!(r.bytes, 16 << 20);
}

#[test]
fn hard_cap_stops_runaway_output() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("zeros.zip");
    evil_zip(&p, &["a.bin", "b.bin"], &vec![0u8; 8 << 20]);
    let opts = ExtractOptions { max_ratio: 10, floor: 0, hard_cap: Some(10 << 20), ..Default::default() };
    let e = extract(&p, &t.path().join("out"), &opts, &mut |_| {}, None).unwrap_err();
    assert!(matches!(e, Error::TooLarge { .. }));
    assert!(!t.path().join("out/b.bin").exists(), "partial file must be removed");
}

#[test]
fn tiny_and_empty_things() {
    for format in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Tiny");
        fs::create_dir_all(root.join("empty-folder")).unwrap();
        fs::write(root.join("zero.txt"), b"").unwrap();
        fs::write(root.join("one.txt"), b"x").unwrap();
        let out = t.path().join(format!("tiny.{}", format.extension()));
        let opts = PackOptions { format, ..Default::default() };
        let rep = pack(&[root.clone()], &out, &opts, &mut |_| {}, None).unwrap();
        assert_eq!(rep.files, 2, "{format:?}");
        let info = inspect(&out, None).unwrap();
        assert_eq!(info.total_size, 1, "{format:?}");
        let dest = t.path().join("x");
        extract(&out, &dest, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
        assert_eq!(fs::metadata(dest.join("Tiny/zero.txt")).unwrap().len(), 0, "{format:?}");
        assert_eq!(fs::read(dest.join("Tiny/one.txt")).unwrap(), b"x", "{format:?}");
        assert!(dest.join("Tiny/empty-folder").is_dir(), "{format:?}");

        // A single empty file on its own.
        let single = t.path().join(format!("single.{}", format.extension()));
        pack(&[root.join("zero.txt")], &single, &opts, &mut |_| {}, None).unwrap();
        let d2 = t.path().join("y");
        extract(&single, &d2, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
        assert_eq!(fs::metadata(d2.join("zero.txt")).unwrap().len(), 0, "{format:?}");
    }
}

#[test]
fn many_small_files() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Many");
    for d in 0..50 {
        let dir = root.join(format!("d{d:02}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            fs::write(dir.join(format!("f{f:03}.txt")), format!("{d}-{f}")).unwrap();
        }
    }
    for format in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let out = t.path().join(format!("many.{}", format.extension()));
        let opts = PackOptions { format, ..Default::default() };
        let rep = pack(&[root.clone()], &out, &opts, &mut |_| {}, None).unwrap();
        assert_eq!(rep.files, 5000, "{format:?}");
        assert_eq!(inspect(&out, None).unwrap().file_count, 5000, "{format:?}");
        let dest = t.path().join(format!("x-{}", format.extension()));
        let r = extract(&out, &dest, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
        assert_eq!(r.files, 5000, "{format:?}");
        assert_eq!(fs::read_to_string(dest.join("Many/d49/f099.txt")).unwrap(), "49-99");
    }
}

#[test]
fn smart_extract_destination() {
    let t = tempfile::tempdir().unwrap();
    let src = sample(t.path());
    let one_folder = t.path().join("one.zip");
    pack(&[src.clone()], &one_folder, &PackOptions::default(), &mut |_| {}, None).unwrap();
    let info = inspect(&one_folder, None).unwrap();
    // "Photos" already exists next to the archive, so it falls back to a new folder.
    let (dest, result) = suggest_dest(&one_folder, &info);
    assert_eq!(dest, t.path().join("one"));
    assert_eq!(result, dest);
    fs::rename(&src, t.path().join("moved")).unwrap();
    let (dest, result) = suggest_dest(&one_folder, &info);
    assert_eq!(dest, t.path());
    assert_eq!(result, t.path().join("Photos"));

    let loose = t.path().join("loose.zip");
    evil_zip(&loose, &["a.txt", "b.txt"], b"x");
    let info = inspect(&loose, None).unwrap();
    fs::create_dir_all(t.path().join("loose")).unwrap();
    let (dest, _) = suggest_dest(&loose, &info);
    assert_eq!(dest, t.path().join("loose (2)"));
}

/// Over 4 GB (needs ZIP64). Slow and writes ~9 GB of temporary data, so run it on demand:
/// `cargo test -p zipkoo-core --release -- --ignored huge`
#[test]
#[ignore]
fn huge_file_over_4gb() {
    use std::io::{Seek, SeekFrom};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Big");
    fs::create_dir_all(&root).unwrap();
    let big = root.join("disk.img");
    let size: u64 = (4 << 30) + (300 << 20);
    {
        let mut f = fs::File::create(&big).unwrap();
        f.set_len(size).unwrap();
        // Real bytes at both ends so a truncated copy would be caught.
        f.write_all(b"ZIPKOO-START").unwrap();
        f.seek(SeekFrom::Start(size - 10)).unwrap();
        f.write_all(b"ZIPKOO-END").unwrap();
    }
    for format in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let out = t.path().join(format!("big.{}", format.extension()));
        let opts = PackOptions { format, level: Level::Fast, ..Default::default() };
        let rep = pack(&[root.clone()], &out, &opts, &mut |_| {}, None).unwrap();
        assert_eq!(rep.input_bytes, size);
        let info = inspect(&out, None).unwrap();
        assert_eq!(info.total_size, size, "{format:?}");
        let dest = t.path().join("x");
        let r = extract(&out, &dest, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
        assert_eq!(r.bytes, size, "{format:?}");
        let got = dest.join("Big/disk.img");
        assert_eq!(fs::metadata(&got).unwrap().len(), size);
        let mut f = fs::File::open(&got).unwrap();
        let mut head = [0u8; 12];
        std::io::Read::read_exact(&mut f, &mut head).unwrap();
        assert_eq!(&head, b"ZIPKOO-START");
        f.seek(SeekFrom::Start(size - 10)).unwrap();
        let mut tail = [0u8; 10];
        std::io::Read::read_exact(&mut f, &mut tail).unwrap();
        assert_eq!(&tail, b"ZIPKOO-END", "{format:?}");
        fs::remove_dir_all(&dest).unwrap();
        fs::remove_file(&out).unwrap();
    }
}

#[test]
fn flags_and_blocks_executables() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("gift.zip");
    evil_zip(&p, &["invoice.pdf.exe", "readme.txt"], b"MZ");
    let info = inspect(&p, None).unwrap();
    assert_eq!(info.executables, vec!["invoice.pdf.exe"]);
    let opts = ExtractOptions { allow_executables: false, ..Default::default() };
    let r = extract(&p, &t.path().join("out"), &opts, &mut |_| {}, None).unwrap();
    assert_eq!(r.files, 1);
    assert_eq!(r.skipped, vec![("invoice.pdf.exe".to_string(), "program blocked".to_string())]);
}

#[test]
fn opens_rar() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let info = inspect(&data.join("solid.rar"), None).unwrap();
    assert_eq!(info.format, Format::Rar);
    assert!(info.file_count > 0);
    let t = tempfile::tempdir().unwrap();
    let r = extract(&data.join("solid.rar"), t.path(), &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!(r.files, info.file_count);

    let crypted = data.join("crypted.rar");
    let e = extract(&crypted, &t.path().join("c1"), &ExtractOptions::default(), &mut |_| {}, None);
    assert!(e.is_err());
    let ok = ExtractOptions { password: Some("unrar"), ..Default::default() };
    let r = extract(&crypted, &t.path().join("c2"), &ok, &mut |_| {}, None).unwrap();
    assert!(r.files > 0);
}

#[test]
fn cancel_leaves_no_partial_archive() {
    use std::sync::atomic::AtomicBool;
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("big");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), "z".repeat(4 << 20)).unwrap();
    let cancel = AtomicBool::new(true);
    let out = t.path().join("c.zip");
    let e = pack(&[src], &out, &PackOptions::default(), &mut |_| {}, Some(&cancel)).unwrap_err();
    assert!(matches!(e, Error::Cancelled), "{e:?}");
    assert!(!out.exists());
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
}
