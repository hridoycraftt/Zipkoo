use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zipkoo_core::*;

fn names(p: &Path) -> Vec<String> {
    let mut v: Vec<String> = inspect(p, None)
        .unwrap()
        .entries
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.name)
        .collect();
    v.sort();
    v
}

fn roundtrip_single(t: &Path, file: &str, format: Format, data: Vec<u8>) {
    let p = t.join(file);
    fs::write(&p, &data).unwrap();
    let info = inspect(&p, None).unwrap();
    assert_eq!(info.format, format, "{file}");
    assert_eq!(names(&p), vec!["report.pdf".to_string()], "{file}");
    assert_eq!(
        peek(&p, "report.pdf", None, 1 << 20).unwrap(),
        b"%PDF-1.7 hello"
    );
    let d = t.join(format!("out-{file}"));
    extract(&p, &d, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!(fs::read(d.join("report.pdf")).unwrap(), b"%PDF-1.7 hello");
    assert!(test(&p, None, &mut |_| {}, None).unwrap().ok(), "{file}");
}

#[test]
fn single_compressed_files() {
    let t = tempfile::tempdir().unwrap();
    let body = b"%PDF-1.7 hello".to_vec();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&body).unwrap();
    roundtrip_single(t.path(), "report.pdf.gz", Format::Gz, gz.finish().unwrap());
    let mut xz = liblzma::write::XzEncoder::new(Vec::new(), 6);
    xz.write_all(&body).unwrap();
    roundtrip_single(t.path(), "report.pdf.xz", Format::Xz, xz.finish().unwrap());
    let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    bz.write_all(&body).unwrap();
    roundtrip_single(
        t.path(),
        "report.pdf.bz2",
        Format::Bz2,
        bz.finish().unwrap(),
    );
    roundtrip_single(
        t.path(),
        "report.pdf.zst",
        Format::Zst,
        zstd::encode_all(&body[..], 3).unwrap(),
    );
    // gzip trailer gives the size.
    assert_eq!(
        inspect(&t.path().join("report.pdf.gz"), None)
            .unwrap()
            .total_size,
        body.len() as u64
    );
}

#[test]
fn tar_inside_plain_gz_is_detected() {
    let t = tempfile::tempdir().unwrap();
    let mut tarball = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_size(5);
    h.set_mode(0o644);
    h.set_cksum();
    tarball
        .append_data(&mut h, "inside/a.txt", &b"hello"[..])
        .unwrap();
    let raw = tarball.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    let p = t.path().join("backup.gz");
    fs::write(&p, gz.finish().unwrap()).unwrap();
    let info = inspect(&p, None).unwrap();
    assert_eq!(info.format, Format::TarGz);
    assert_eq!(names(&p), vec!["inside/a.txt"]);
}

#[test]
fn zipx_opens_as_zip() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Docs");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), "x").unwrap();
    let z = t.path().join("a.zip");
    pack(&[src], &z, &PackOptions::default(), &mut |_| {}, None).unwrap();
    let zx = t.path().join("a.zipx");
    fs::rename(&z, &zx).unwrap();
    assert_eq!(inspect(&zx, None).unwrap().format, Format::Zip);
    assert_eq!(names(&zx), vec!["Docs/a.txt"]);
}

#[test]
fn iso_with_joliet_names() {
    let t = tempfile::tempdir().unwrap();
    let gz =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sample.iso.gz")).unwrap();
    let mut iso = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut iso)
        .unwrap();
    let p = t.path().join("disc.iso");
    fs::write(&p, iso).unwrap();
    let info = inspect(&p, None).unwrap();
    assert_eq!(info.format, Format::Iso);
    assert_eq!(
        names(&p),
        vec!["Docs/Deep/inner.txt", "Read Me Long Name.txt"]
    );
    assert_eq!(
        peek(&p, "Read Me Long Name.txt", None, 1000).unwrap(),
        b"hello from iso\n"
    );
    let d = t.path().join("out");
    extract(&p, &d, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!(
        fs::read(d.join("Docs/Deep/inner.txt")).unwrap(),
        b"deep file\n"
    );
    assert!(test(&p, None, &mut |_| {}, None).unwrap().ok());
}

#[test]
fn cab_files() {
    let t = tempfile::tempdir().unwrap();
    let mut b = cab::CabinetBuilder::new();
    {
        let folder = b.add_folder(cab::CompressionType::MsZip);
        folder.add_file("setup.ini");
        folder.add_file("data\\readme.txt");
    }
    let mut w = b.build(std::io::Cursor::new(Vec::new())).unwrap();
    let mut bodies = [&b"[setup]\nok=1\n"[..], &b"cab readme"[..]].into_iter();
    while let Some(mut fw) = w.next_file().unwrap() {
        fw.write_all(bodies.next().unwrap()).unwrap();
    }
    let p = t.path().join("pack.cab");
    fs::write(&p, w.finish().unwrap().into_inner()).unwrap();
    assert_eq!(inspect(&p, None).unwrap().format, Format::Cab);
    assert_eq!(names(&p), vec!["data/readme.txt", "setup.ini"]);
    let d = t.path().join("out");
    extract(&p, &d, &ExtractOptions::default(), &mut |_| {}, None).unwrap();
    assert_eq!(fs::read(d.join("data/readme.txt")).unwrap(), b"cab readme");
}

/// CRC-16/ARC, as used by LHA.
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// A level-0 LHA archive with stored (-lh0-) entries.
fn lzh(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in files {
        let mut h = Vec::new();
        h.extend_from_slice(b"-lh0-");
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&0x5A21_6000u32.to_le_bytes()); // DOS date/time
        h.push(0x20);
        h.push(0);
        h.push(name.len() as u8);
        h.extend_from_slice(name.as_bytes());
        h.extend_from_slice(&crc16(data).to_le_bytes());
        let sum = h.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        out.push(h.len() as u8);
        out.push(sum);
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
    }
    out.push(0);
    out
}

#[test]
fn lzh_files() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("old.lzh");
    fs::write(
        &p,
        lzh(&[
            ("GAME\\README.TXT", b"retro"),
            ("GAME\\RUN.BAT", b"@echo off"),
        ]),
    )
    .unwrap();
    assert_eq!(inspect(&p, None).unwrap().format, Format::Lzh);
    assert_eq!(names(&p), vec!["GAME/README.TXT", "GAME/RUN.BAT"]);
    assert_eq!(peek(&p, "GAME/README.TXT", None, 100).unwrap(), b"retro");
    assert!(test(&p, None, &mut |_| {}, None).unwrap().ok());
}

#[test]
fn rar_parts_open_from_any_volume() {
    let t = tempfile::tempdir().unwrap();
    for n in ["Movie.part01.rar", "Movie.part02.rar", "Movie.part03.rar"] {
        fs::write(t.path().join(n), b"x").unwrap();
    }
    let first = zipkoo_core::inspect::rar_first_volume(&t.path().join("Movie.part03.rar"));
    assert_eq!(first, Some(t.path().join("Movie.part01.rar")));
    assert_eq!(
        zipkoo_core::inspect::rar_first_volume(&t.path().join("Movie.part01.rar")),
        None
    );
    assert_eq!(
        zipkoo_core::inspect::rar_first_volume(&PathBuf::from("/x/plain.rar")),
        None
    );
}
