use std::fs;
use std::io::Write;
use std::path::Path;
use zipkoo_core::*;

fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    vec![
        ("a.txt".into(), b"first file ".repeat(300)),
        (
            "b.bin".into(),
            (0..120_000u32)
                .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
                .collect(),
        ),
        ("c.txt".into(), b"third file ".repeat(300)),
    ]
    .into_iter()
    .map(|(n, d)| {
        fs::write(dir.join(&n), &d).unwrap();
        (n, d)
    })
    .collect()
}

fn names(p: &Path) -> Vec<String> {
    let mut v: Vec<String> = inspect(p, None)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect();
    v.sort();
    v
}

#[test]
fn salvage_cut_off_and_damaged() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let data = files(&src);
    let z = t.path().join("x.zip");
    let mut w = zip::ZipWriter::new(fs::File::create(&z).unwrap());
    for (n, d) in &data {
        w.start_file(n.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(d).unwrap();
    }
    w.finish().unwrap();
    let mut bytes = fs::read(&z).unwrap();
    // Damage the middle file's data and cut off the central directory.
    let b_at = bytes.windows(5).position(|w| w == b"b.bin").unwrap() + 40;
    for x in &mut bytes[b_at..b_at + 64] {
        *x ^= 0xFF;
    }
    let cut = bytes.windows(4).rposition(|w| w == b"PK\x01\x02").unwrap();
    let broken = t.path().join("broken.zip");
    fs::write(&broken, &bytes[..cut - 10]).unwrap();
    assert!(inspect(&broken, None).is_err(), "normal open must fail");

    let out = salvage::recovered_path(&broken);
    assert_eq!(out.file_name().unwrap(), "broken (recovered).zip");
    let r = salvage::salvage(&broken, &out, &mut |_| {}, None).unwrap();
    assert_eq!(r.recovered, 2, "{r:?}");
    assert!(r.lost.iter().any(|(n, _)| n == "b.bin"), "{r:?}");
    assert_eq!(names(&out), vec!["a.txt", "c.txt"]);
    assert_eq!(peek(&out, "c.txt", None, 1 << 20).unwrap(), data[2].1);
}

#[test]
fn salvage_streamed_zip_with_data_descriptors() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let data = files(&src);
    // A ZIP written to a non-seekable stream: sizes and CRCs come after each file.
    let mut w = zip::ZipWriter::new_stream(Vec::new());
    for (n, d) in &data {
        w.start_file(n.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(d).unwrap();
    }
    let bytes = w.finish().unwrap().into_inner();
    let cut = bytes.windows(4).rposition(|w| w == b"PK\x01\x02").unwrap();
    let p = t.path().join("stream.zip");
    fs::write(&p, &bytes[..cut]).unwrap();
    let out = t.path().join("rec.zip");
    let r = salvage::salvage(&p, &out, &mut |_| {}, None).unwrap();
    assert_eq!(r.recovered, 3, "{r:?}");
    assert_eq!(peek(&out, "b.bin", None, 1 << 20).unwrap(), data[1].1);
}

/// A one-file stored ZIP with a Shift-JIS name and no UTF-8 flag, like old Japanese Windows tools wrote.
fn legacy_zip(name: &[u8], body: &[u8]) -> Vec<u8> {
    let crc = crc32fast::hash(body);
    let mut v = Vec::new();
    let local = |v: &mut Vec<u8>| {
        v.extend_from_slice(b"PK\x03\x04");
        v.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        v.extend_from_slice(&crc.to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(name);
    };
    local(&mut v);
    v.extend_from_slice(body);
    let cd_at = v.len() as u32;
    v.extend_from_slice(b"PK\x01\x02");
    v.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
    v.extend_from_slice(&crc.to_le_bytes());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(&(name.len() as u16).to_le_bytes());
    v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    v.extend_from_slice(name);
    let cd_len = v.len() as u32 - cd_at;
    v.extend_from_slice(b"PK\x05\x06");
    v.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    v.extend_from_slice(&cd_len.to_le_bytes());
    v.extend_from_slice(&cd_at.to_le_bytes());
    v.extend_from_slice(&[0, 0]);
    v
}

#[test]
fn legacy_japanese_names_are_readable() {
    let t = tempfile::tempdir().unwrap();
    let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("会議資料フォルダ/報告書（最終）.txt");
    let p = t.path().join("old.zip");
    fs::write(&p, legacy_zip(&sjis, b"hello")).unwrap();
    assert_eq!(names(&p), vec!["会議資料フォルダ/報告書（最終）.txt"]);
    assert_eq!(
        peek(&p, "会議資料フォルダ/報告書（最終）.txt", None, 100).unwrap(),
        b"hello"
    );
    let d = t.path().join("out");
    extract(&p, &d, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!(
        fs::read(d.join("会議資料フォルダ/報告書（最終）.txt")).unwrap(),
        b"hello"
    );
    // Editing finds the same name.
    let r = edit::edit(
        &p,
        &edit::Change::Delete {
            names: vec!["会議資料フォルダ/報告書（最終）.txt".into()],
        },
        None,
        &mut |_| {},
        None,
    )
    .unwrap();
    assert_eq!(r.deleted, 1);
}
