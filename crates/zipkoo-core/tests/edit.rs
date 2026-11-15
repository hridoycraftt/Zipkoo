use std::fs;
use std::path::{Path, PathBuf};
use zipkoo_core::edit::{edit, Change};
use zipkoo_core::*;

fn tree(dir: &Path) -> PathBuf {
    let root = dir.join("Project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("README.md"), "readme").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn x() {}").unwrap();
    root
}

fn names(p: &Path, pw: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = inspect(p, pw)
        .unwrap()
        .entries
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.name)
        .collect();
    v.sort();
    v
}

fn new_file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, body).unwrap();
    p
}

fn run_suite(format: Format, password: Option<&str>) {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arc = t.path().join(format!("a.{}", format.extension()));
    pack(
        &[src],
        &arc,
        &PackOptions {
            format,
            password: password.map(String::from),
            hide_names: password.is_some(),
            ..Default::default()
        },
        &mut |_| {},
        None,
    )
    .unwrap();
    let extra = t.path().join("extra");
    fs::create_dir_all(&extra).unwrap();

    // Add into a folder.
    let f = new_file(&extra, "notes.txt", "hello");
    let r = edit(
        &arc,
        &Change::Add {
            inputs: vec![f],
            folder: "Project/src".into(),
            replace: false,
        },
        password,
        &mut |_| {},
        None,
    )
    .unwrap();
    assert_eq!(r.added, 1, "{format:?}");
    assert!(
        names(&arc, password).contains(&"Project/src/notes.txt".to_string()),
        "{format:?}"
    );
    assert_eq!(
        peek(&arc, "Project/src/notes.txt", password, 100).unwrap(),
        b"hello"
    );

    // Same name again: skipped unless replace.
    let f = new_file(&extra, "notes.txt", "v2");
    let r = edit(
        &arc,
        &Change::Add {
            inputs: vec![f.clone()],
            folder: "Project/src/".into(),
            replace: false,
        },
        password,
        &mut |_| {},
        None,
    )
    .unwrap();
    assert_eq!((r.added, r.skipped.len()), (0, 1), "{format:?}");
    let r = edit(
        &arc,
        &Change::Add {
            inputs: vec![f],
            folder: "Project/src".into(),
            replace: true,
        },
        password,
        &mut |_| {},
        None,
    )
    .unwrap();
    assert_eq!(r.replaced, 1, "{format:?}");
    assert_eq!(
        peek(&arc, "Project/src/notes.txt", password, 100).unwrap(),
        b"v2"
    );

    // Rename a file and a folder.
    edit(
        &arc,
        &Change::Rename {
            from: "Project/README.md".into(),
            to: "Project/READ ME.md".into(),
        },
        password,
        &mut |_| {},
        None,
    )
    .unwrap();
    edit(
        &arc,
        &Change::Rename {
            from: "Project/src/".into(),
            to: "Project/code".into(),
        },
        password,
        &mut |_| {},
        None,
    )
    .unwrap();
    let n = names(&arc, password);
    assert!(
        n.contains(&"Project/READ ME.md".to_string())
            && n.contains(&"Project/code/main.rs".to_string()),
        "{format:?} {n:?}"
    );
    assert!(
        !n.iter().any(|x| x.starts_with("Project/src/")),
        "{format:?}"
    );
    assert!(
        edit(
            &arc,
            &Change::Rename {
                from: "Project/code/lib.rs".into(),
                to: "Project/code/main.rs".into()
            },
            password,
            &mut |_| {},
            None
        )
        .is_err(),
        "rename onto existing must fail"
    );
    assert!(edit(
        &arc,
        &Change::Rename {
            from: "Project/code/lib.rs".into(),
            to: "../escape.rs".into()
        },
        password,
        &mut |_| {},
        None
    )
    .is_err());

    if password.is_some() {
        let enc = inspect(&arc, None).map(|i| i.encrypted).unwrap_or(true);
        assert!(enc, "{format:?} must stay locked after edits");
    }

    // Delete a file and a folder.
    edit(
        &arc,
        &Change::Delete {
            names: vec!["Project/READ ME.md".into(), "Project/code/".into()],
        },
        password,
        &mut |_| {},
        None,
    )
    .unwrap();
    assert!(
        names(&arc, password).is_empty(),
        "{format:?} {:?}",
        names(&arc, password)
    );

    // Still opens and extracts.
    extract(
        &arc,
        &t.path().join("x"),
        &ExtractOptions {
            password,
            ..Default::default()
        },
        &mut |_| {},
        None,
    )
    .unwrap();
}

#[test]
fn edit_zip() {
    run_suite(Format::Zip, None)
}
#[test]
fn edit_zip_locked() {
    run_suite(Format::Zip, Some("pw-123456"))
}
#[test]
fn edit_7z() {
    run_suite(Format::SevenZ, None)
}
#[test]
fn edit_7z_locked_hidden_names() {
    run_suite(Format::SevenZ, Some("pw-123456"))
}
#[test]
fn edit_tar_zst() {
    run_suite(Format::TarZst, None)
}

#[test]
fn edit_keeps_zip_history_and_rebuilds_repair_data() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arc = t.path().join("a.zip");
    pack(
        &[src.clone()],
        &arc,
        &PackOptions::default(),
        &mut |_| {},
        None,
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2100));
    fs::write(src.join("README.md"), "v2").unwrap();
    update::update(
        &arc,
        &[src],
        &update::UpdateOptions::default(),
        &mut |_| {},
        None,
    )
    .unwrap();
    heal::create(&arc, 10, &mut |_| {}, None).unwrap();
    edit(
        &arc,
        &Change::Delete {
            names: vec!["Project/src/".into()],
        },
        None,
        &mut |_| {},
        None,
    )
    .unwrap();
    assert_eq!(
        update::read_manifest(&arc).unwrap().versions.len(),
        1,
        "history kept"
    );
    let all: Vec<String> = inspect(&arc, None)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(
        all.iter().any(|n| n.starts_with(".zipkoo/history/")),
        "old versions kept: {all:?}"
    );
    assert!(
        heal::check(&arc, false, &mut |_| {}, None)
            .unwrap()
            .healthy(),
        "repair data rebuilt"
    );
}

#[test]
fn rar_is_read_only() {
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/solid.rar");
    let t = tempfile::tempdir().unwrap();
    let copy = t.path().join("x.rar");
    fs::copy(rar, &copy).unwrap();
    assert!(matches!(
        edit(
            &copy,
            &Change::Delete {
                names: vec!["a".into()]
            },
            None,
            &mut |_| {},
            None
        ),
        Err(Error::CreateNotSupported(_))
    ));
}

#[test]
fn locked_zip_needs_right_password_to_edit() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arc = t.path().join("a.zip");
    pack(
        &[src],
        &arc,
        &PackOptions {
            password: Some("right-pw".into()),
            ..Default::default()
        },
        &mut |_| {},
        None,
    )
    .unwrap();
    assert!(matches!(
        edit(
            &arc,
            &Change::Delete {
                names: vec!["Project/README.md".into()]
            },
            None,
            &mut |_| {},
            None
        ),
        Err(Error::PasswordRequired)
    ));
    assert!(matches!(
        edit(
            &arc,
            &Change::Delete {
                names: vec!["Project/README.md".into()]
            },
            Some("nope"),
            &mut |_| {},
            None
        ),
        Err(Error::WrongPassword)
    ));
}
