use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zipkoo_core::*;

fn tree(dir: &Path) -> PathBuf {
    let root = dir.join("Project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("README.md"), "# Project\n".repeat(200)).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n".repeat(300)).unwrap();
    fs::write(root.join("logo.png"), (0..50_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect::<Vec<_>>()).unwrap();
    root
}

fn names(p: &Path, pw: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = inspect(p, pw).unwrap().entries.into_iter().filter(|e| !e.is_dir).map(|e| e.name).collect();
    v.sort();
    v
}

// ---------- Batch Convert (12) ----------

#[test]
fn convert_between_formats() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let zip = t.path().join("p.zip");
    pack(&[src.clone()], &zip, &PackOptions { password: Some("pw1234".into()), ..Default::default() }, &mut |_| {}, None).unwrap();
    for to in [Format::SevenZ, Format::TarZst] {
        let out = convert::converted_path(&zip, to);
        assert_eq!(out.extension().unwrap(), if to == Format::SevenZ { "7z" } else { "zst" });
        let opts = convert::ConvertOptions { password: Some("pw1234"), pack: PackOptions { format: to, ..Default::default() } };
        let r = convert::convert(&zip, &out, &opts, &mut |_| {}, None).unwrap();
        assert_eq!(r.files, 3);
        assert_eq!(names(&out, None), names(&zip, None));
        assert!(!t.path().join(".zipkoo-convert").exists(), "temp folder must be cleaned up");
    }
    // RAR fixture to ZIP.
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/solid.rar");
    let out = t.path().join("from-rar.zip");
    let opts = convert::ConvertOptions { password: None, pack: PackOptions::default() };
    convert::convert(&rar, &out, &opts, &mut |_| {}, None).unwrap();
    assert_eq!(inspect(&out, None).unwrap().file_count, inspect(&rar, None).unwrap().file_count);
    // Never overwrites: a second converted_path picks a new name.
    fs::write(t.path().join("p.7z"), b"x").unwrap();
    assert_eq!(convert::converted_path(&zip, Format::SevenZ), t.path().join("p (2).7z"));
}

// ---------- Smart Update (7) ----------

#[test]
fn smart_update_adds_only_changes_and_keeps_history() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let zip = t.path().join("p.zip");
    pack(&[src.clone()], &zip, &PackOptions::default(), &mut |_| {}, None).unwrap();

    // Nothing changed.
    let r = update::update(&zip, &[src.clone()], &update::UpdateOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!((r.added, r.changed, r.removed, r.unchanged, r.bytes_packed), (0, 0, 0, 3, 0));

    // Edit one file, add one, delete one.
    std::thread::sleep(std::time::Duration::from_millis(2100)); // ZIP times have 2 s steps
    fs::write(src.join("README.md"), "# Project v2\n").unwrap();
    fs::write(src.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    fs::remove_file(src.join("logo.png")).unwrap();
    let opts = update::UpdateOptions { remove_missing: true, ..Default::default() };
    let r = update::update(&zip, &[src.clone()], &opts, &mut |_| {}, None).unwrap();
    assert_eq!((r.added, r.changed, r.removed, r.unchanged), (1, 1, 1, 1));
    assert_eq!(r.bytes_packed, "# Project v2\n".len() as u64 + "pub fn x() {}\n".len() as u64);
    assert_eq!(r.version, 2);

    let all = names(&zip, None);
    assert!(all.contains(&"Project/src/lib.rs".to_string()));
    assert!(!all.contains(&"Project/logo.png".to_string()));
    assert!(all.contains(&".zipkoo/history/v1/Project/README.md".to_string()));
    assert!(all.contains(&".zipkoo/history/v1/Project/logo.png".to_string()));
    let old = peek(&zip, ".zipkoo/history/v1/Project/README.md", None, 1 << 20).unwrap();
    assert!(old.starts_with(b"# Project\n# Project"));
    let new = peek(&zip, "Project/README.md", None, 1 << 20).unwrap();
    assert_eq!(new, b"# Project v2\n");

    let m = update::read_manifest(&zip).unwrap();
    assert_eq!(m.versions.len(), 2);
    assert_eq!(m.versions[1].changed, vec!["Project/README.md"]);
    assert_eq!(m.versions[1].removed, vec!["Project/logo.png"]);
}

#[test]
fn smart_update_locked_archive() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let zip = t.path().join("p.zip");
    pack(&[src.clone()], &zip, &PackOptions { password: Some("secret1".into()), ..Default::default() }, &mut |_| {}, None).unwrap();
    fs::write(src.join("new.txt"), "hello").unwrap();
    assert!(matches!(update::update(&zip, &[src.clone()], &update::UpdateOptions::default(), &mut |_| {}, None), Err(Error::PasswordRequired)));
    let opts = update::UpdateOptions { password: Some("secret1"), ..Default::default() };
    let r = update::update(&zip, &[src.clone()], &opts, &mut |_| {}, None).unwrap();
    assert_eq!(r.added, 1);
    assert_eq!(peek(&zip, "Project/new.txt", Some("secret1"), 100).unwrap(), b"hello");
    assert!(inspect(&zip, None).unwrap().entries.iter().find(|e| e.name == "Project/new.txt").unwrap().encrypted);
}

#[test]
fn timestamps_survive_round_trip() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    filetime::set_file_mtime(src.join("README.md"), filetime::FileTime::from_system_time(past)).unwrap();
    for f in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let out = t.path().join(format!("t.{}", f.extension()));
        pack(&[src.clone()], &out, &PackOptions { format: f, ..Default::default() }, &mut |_| {}, None).unwrap();
        let d = t.path().join(format!("x-{}", f.extension()));
        extract(&out, &d, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
        let got = fs::metadata(d.join("Project/README.md")).unwrap().modified().unwrap();
        assert_eq!(got, past, "{f:?}");
    }
}

// ---------- Self-heal (8) ----------

fn damage(p: &Path, at: u64, len: usize) {
    let mut f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.seek(SeekFrom::Start(at)).unwrap();
    f.write_all(&vec![0xAA; len]).unwrap();
}

fn archive_with_repair(t: &Path, percent: u8) -> (PathBuf, Vec<u8>) {
    let root = t.join("Data");
    fs::create_dir_all(&root).unwrap();
    // ~3 MB of incompressible data so the archive is big enough to have many blocks.
    let noise: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761).rotate_left(i % 13) >> 11) as u8).collect();
    fs::write(root.join("noise.bin"), &noise).unwrap();
    let zip = t.join("d.zip");
    pack(&[root], &zip, &PackOptions { smart: false, ..Default::default() }, &mut |_| {}, None).unwrap();
    let rec = heal::create(&zip, percent, &mut |_| {}, None).unwrap();
    assert!(rec.path.exists());
    let ratio = rec.size as f64 / fs::metadata(&zip).unwrap().len() as f64;
    assert!(ratio < percent as f64 / 100.0 * 1.6 + 0.02, "repair file too big: {ratio}");
    (zip.clone(), fs::read(&zip).unwrap())
}

#[test]
fn heal_fixes_scattered_and_burst_damage() {
    let t = tempfile::tempdir().unwrap();
    let (zip, original) = archive_with_repair(t.path(), 10);
    assert!(heal::check(&zip, false, &mut |_| {}, None).unwrap().healthy());

    // A burst of 150 KB (≈5% of the file) plus a few scattered hits.
    damage(&zip, 400_000, 150_000);
    damage(&zip, 10, 3);
    damage(&zip, 2_500_000, 1);
    let h = heal::check(&zip, false, &mut |_| {}, None).unwrap();
    assert!(h.damaged > 0 && h.repairable(), "{h:?}");
    assert!(extract(&zip, &t.path().join("bad"), &ExtractOptions::default(), &mut |_| {}, None).is_err());

    let h = heal::check(&zip, true, &mut |_| {}, None).unwrap();
    assert_eq!(h.repaired, h.damaged);
    assert_eq!(fs::read(&zip).unwrap(), original, "repaired file must match the original byte for byte");
    extract(&zip, &t.path().join("good"), &ExtractOptions::default(), &mut |_| {}, None).unwrap();
}

#[test]
fn heal_fixes_cut_off_download() {
    let t = tempfile::tempdir().unwrap();
    let (zip, original) = archive_with_repair(t.path(), 10);
    let len = original.len() as u64;
    fs::OpenOptions::new().write(true).open(&zip).unwrap().set_len(len - 120_000).unwrap();
    let h = heal::check(&zip, false, &mut |_| {}, None).unwrap();
    assert!(!h.size_ok && h.repairable(), "{h:?}");
    heal::check(&zip, true, &mut |_| {}, None).unwrap();
    assert_eq!(fs::read(&zip).unwrap(), original);
}

#[test]
fn heal_reports_too_much_damage() {
    let t = tempfile::tempdir().unwrap();
    let (zip, _) = archive_with_repair(t.path(), 5);
    damage(&zip, 0, 1_500_000);
    let h = heal::check(&zip, true, &mut |_| {}, None).unwrap();
    assert!(!h.repairable(), "{h:?}");
    assert_eq!(h.repaired, 0, "never half-repair");
}
