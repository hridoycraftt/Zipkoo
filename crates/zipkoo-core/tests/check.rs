use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zipkoo_core::*;

fn tree(dir: &Path) -> PathBuf {
    let root = dir.join("Data");
    fs::create_dir_all(&root).unwrap();
    // Incompressible-ish so damage lands inside file data.
    let noise: Vec<u8> = (0..400_000u32).map(|i| (i.wrapping_mul(2654435761).rotate_left(i % 13) >> 11) as u8).collect();
    fs::write(root.join("a.bin"), &noise).unwrap();
    fs::write(root.join("b.txt"), "hello").unwrap();
    root
}

fn corrupt(p: &Path) {
    let len = fs::metadata(p).unwrap().len();
    let mut f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.seek(SeekFrom::Start(len / 2)).unwrap();
    f.write_all(&[0x55; 64]).unwrap();
}

#[test]
fn test_reports_good_and_damaged() {
    for format in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let t = tempfile::tempdir().unwrap();
        let src = tree(t.path());
        let arc = t.path().join(format!("x.{}", format.extension()));
        pack(&[src], &arc, &PackOptions { format, smart: false, ..Default::default() }, &mut |_| {}, None).unwrap();
        let r = test(&arc, None, &mut |_| {}, None).unwrap();
        assert!(r.ok() && r.files == 2, "{format:?} {r:?}");
        corrupt(&arc);
        let bad = test(&arc, None, &mut |_| {}, None);
        let damaged = match bad {
            Ok(r) => !r.ok(),
            Err(_) => true,
        };
        assert!(damaged, "{format:?} damage must be reported");
    }
}

#[test]
fn test_locked_and_rar() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arc = t.path().join("x.zip");
    pack(&[src], &arc, &PackOptions { password: Some("pw-123456".into()), ..Default::default() }, &mut |_| {}, None).unwrap();
    assert!(matches!(test(&arc, None, &mut |_| {}, None), Err(Error::PasswordRequired)));
    assert!(test(&arc, Some("pw-123456"), &mut |_| {}, None).unwrap().ok());
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/solid.rar");
    let r = test(&rar, None, &mut |_| {}, None).unwrap();
    assert!(r.ok() && r.files > 0, "{r:?}");
}

#[test]
fn conflicts_listed_with_strip() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arc = t.path().join("x.zip");
    pack(&[src], &arc, &PackOptions::default(), &mut |_| {}, None).unwrap();
    let dest = t.path().join("out");
    fs::create_dir_all(dest.join("Data")).unwrap();
    fs::write(dest.join("Data/b.txt"), "mine").unwrap();
    assert_eq!(conflicts(&arc, &dest, &ExtractOptions::default()).unwrap(), vec!["Data/b.txt"]);
    let flat = t.path().join("flat");
    fs::create_dir_all(&flat).unwrap();
    fs::write(flat.join("a.bin"), "x").unwrap();
    let opts = ExtractOptions { strip: Some("Data/".into()), ..Default::default() };
    assert_eq!(conflicts(&arc, &flat, &opts).unwrap(), vec!["a.bin"]);
    let only = ExtractOptions { only: vec!["Data/a.bin".into()], ..Default::default() };
    assert!(conflicts(&arc, &dest, &only).unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn mark_of_the_web_is_copied() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arc = t.path().join("download.zip");
    pack(&[src], &arc, &PackOptions::default(), &mut |_| {}, None).unwrap();
    let zone = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/download.zip\r\n";
    fs::write(format!("{}:Zone.Identifier", arc.display()), zone).unwrap();
    let out = t.path().join("out");
    extract(&arc, &out, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    let got = fs::read_to_string(format!("{}:Zone.Identifier", out.join("Data/b.txt").display())).unwrap();
    assert!(got.contains("ZoneId=3"));

    // Local archives (no mark) produce unmarked files.
    let local = t.path().join("local.zip");
    fs::copy(&arc, &local).unwrap();
    let _ = fs::remove_file(format!("{}:Zone.Identifier", local.display()));
    let out2 = t.path().join("out2");
    extract(&local, &out2, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert!(fs::read(format!("{}:Zone.Identifier", out2.join("Data/b.txt").display())).is_err());
}
