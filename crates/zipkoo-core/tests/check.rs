use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zipkoo_core::*;

fn tree(dir: &Path) -> PathBuf {
    let root = dir.join("Data");
    fs::create_dir_all(&root).unwrap();
    // Incompressible-ish so damage lands inside file data.
    let noise: Vec<u8> = (0..400_000u32)
        .map(|i| (i.wrapping_mul(2654435761).rotate_left(i % 13) >> 11) as u8)
        .collect();
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
        pack(
            &[src],
            &arc,
            &PackOptions {
                format,
                smart: false,
                ..Default::default()
            },
            &mut |_| {},
            None,
        )
        .unwrap();
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
    pack(
        &[src],
        &arc,
        &PackOptions {
            password: Some("pw-123456".into()),
            ..Default::default()
        },
        &mut |_| {},
        None,
    )
    .unwrap();
    assert!(matches!(
        test(&arc, None, &mut |_| {}, None),
        Err(Error::PasswordRequired)
    ));
    assert!(test(&arc, Some("pw-123456"), &mut |_| {}, None)
        .unwrap()
        .ok());
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
    assert_eq!(
        conflicts(&arc, &dest, &ExtractOptions::default()).unwrap(),
        vec!["Data/b.txt"]
    );
    let flat = t.path().join("flat");
    fs::create_dir_all(&flat).unwrap();
    fs::write(flat.join("a.bin"), "x").unwrap();
    let opts = ExtractOptions {
        strip: Some("Data/".into()),
        ..Default::default()
    };
    assert_eq!(conflicts(&arc, &flat, &opts).unwrap(), vec!["a.bin"]);
    let only = ExtractOptions {
        only: vec!["Data/a.bin".into()],
        ..Default::default()
    };
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
    let got = fs::read_to_string(format!(
        "{}:Zone.Identifier",
        out.join("Data/b.txt").display()
    ))
    .unwrap();
    assert!(got.contains("ZoneId=3"));

    // Local archives (no mark) produce unmarked files.
    let local = t.path().join("local.zip");
    fs::copy(&arc, &local).unwrap();
    let _ = fs::remove_file(format!("{}:Zone.Identifier", local.display()));
    let out2 = t.path().join("out2");
    extract(&local, &out2, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert!(fs::read(format!(
        "{}:Zone.Identifier",
        out2.join("Data/b.txt").display()
    ))
    .is_err());
}

#[test]
fn password_ok_for_each_format() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    for (format, hide) in [
        (Format::Zip, false),
        (Format::SevenZ, false),
        (Format::SevenZ, true),
    ] {
        let arc = t.path().join(format!("p{hide}.{}", format.extension()));
        pack(
            &[src.clone()],
            &arc,
            &PackOptions {
                format,
                password: Some("right-1".into()),
                hide_names: hide,
                ..Default::default()
            },
            &mut |_| {},
            None,
        )
        .unwrap();
        assert!(
            inspect::password_ok(&arc, "right-1").unwrap(),
            "{format:?} hide={hide}"
        );
        assert!(
            !inspect::password_ok(&arc, "wrong-2").unwrap(),
            "{format:?} hide={hide}"
        );
    }
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/crypted.rar");
    assert!(inspect::password_ok(&rar, "unrar").unwrap());
    assert!(!inspect::password_ok(&rar, "nope").unwrap());
}

#[cfg(unix)]
#[test]
fn virus_scan_with_clamav_compatible_scanner() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let fake = t.path().join("clamscan");
    // Behaves like clamscan: reports a "virus" in any file named evil.*
    fs::write(&fake, "#!/bin/sh\nfor a in \"$@\"; do last=\"$a\"; done\nhit=$(find \"$last\" -name 'evil.*' | head -1)\nif [ -n \"$hit\" ]; then echo \"$hit: Eicar-Signature FOUND\"; exit 1; fi\nexit 0\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("ZIPKOO_CLAMSCAN", &fake);
    let clean = t.path().join("clean");
    fs::create_dir_all(&clean).unwrap();
    fs::write(clean.join("ok.txt"), "fine").unwrap();
    let r = scan::scan(&clean).unwrap();
    assert!(r.clean && r.engine == "ClamAV", "{r:?}");
    fs::write(clean.join("evil.com"), "X5O!P%@AP").unwrap();
    let r = scan::scan(&clean).unwrap();
    assert!(
        !r.clean && r.threats == vec!["evil.com: Eicar-Signature".to_string()],
        "{r:?}"
    );
    assert_eq!(scan::available().as_deref(), Some("ClamAV"));
}
