use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use zipkoo_core::{sfx, PackOptions};

fn stub() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zipkoo-sfx"))
}

fn sample(dir: &Path) -> PathBuf {
    let root = dir.join("Gift");
    fs::create_dir_all(root.join("inner/empty")).unwrap();
    fs::write(root.join("hello.txt"), "hi from zipkoo\n".repeat(1000)).unwrap();
    fs::write(root.join("inner/zero.bin"), b"").unwrap();
    root
}

fn build(dir: &Path, password: Option<&str>) -> PathBuf {
    let src = sample(dir);
    let exe = dir.join(if cfg!(windows) { "Gift.exe" } else { "Gift.run" });
    let opts = PackOptions { password: password.map(String::from), ..Default::default() };
    let r = sfx::build(&stub(), &[src], &exe, &opts, &mut |_| {}, None).unwrap();
    assert_eq!(r.outputs, vec![exe.clone()]);
    assert!(!dir.join("Gift.sfx-payload.zip").exists());
    exe
}

#[test]
fn unpacks_itself() {
    let t = tempfile::tempdir().unwrap();
    let exe = build(t.path(), None);
    let out = t.path().join("out");
    let st = Command::new(&exe).args(["--silent", "--to"]).arg(&out).status().unwrap();
    assert!(st.success());
    assert_eq!(fs::read_to_string(out.join("Gift/hello.txt")).unwrap(), "hi from zipkoo\n".repeat(1000));
    assert_eq!(fs::metadata(out.join("Gift/inner/zero.bin")).unwrap().len(), 0);
    assert!(out.join("Gift/inner/empty").is_dir());
}

#[test]
fn locked_needs_right_password() {
    let t = tempfile::tempdir().unwrap();
    let exe = build(t.path(), Some("open-sesame"));
    let out = t.path().join("out");
    let none = Command::new(&exe).args(["--silent", "--to"]).arg(&out).output().unwrap();
    assert!(!none.status.success());
    let wrong = Command::new(&exe).args(["--silent", "--password", "nope", "--to"]).arg(&out).output().unwrap();
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("Wrong password"), "{}", String::from_utf8_lossy(&wrong.stderr));
    let ok = Command::new(&exe).args(["--silent", "--password", "open-sesame", "--to"]).arg(&out).status().unwrap();
    assert!(ok.success());
    assert!(out.join("Gift/hello.txt").exists());
}

#[test]
fn plain_stub_reports_damage() {
    let out = Command::new(stub()).arg("--silent").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("damaged"));
}

#[test]
fn payload_is_found_by_core() {
    let t = tempfile::tempdir().unwrap();
    let exe = build(t.path(), None);
    let mut f = fs::File::open(&exe).unwrap();
    let (off, len) = sfx::locate_payload(&mut f).unwrap().unwrap();
    assert_eq!(off, fs::metadata(stub()).unwrap().len());
    assert!(len > 0);
}

#[test]
fn refuses_exe_over_4gb_before_packing() {
    let t = tempfile::tempdir().unwrap();
    let big = t.path().join("big.bin");
    // Sparse 20 GB file: takes no real disk space.
    fs::File::create(&big).unwrap().set_len(20 << 30).unwrap();
    let e = sfx::build(&stub(), &[big], &t.path().join("big.exe"), &PackOptions::default(), &mut |_| {}, None).unwrap_err();
    assert!(matches!(e, zipkoo_core::Error::SfxTooBig), "{e:?}");
    assert!(!t.path().join("big.exe").exists());
}
