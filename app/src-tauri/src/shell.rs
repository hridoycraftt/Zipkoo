//! Right-click menu integration.
//! Windows: per-user registry verbs (no admin rights). macOS: Finder Quick Actions in ~/Library/Services.
//! Every menu item runs `Zipkoo --action <id> <paths…>`; the single-instance plugin hands those
//! to the running app, which batches them (see `Inbox`).

#[derive(Clone, Copy)]
pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
}

pub const FILE_ITEMS: &[Item] = &[
    Item { id: "zip-here", label: "Zip here" },
    Item { id: "lock", label: "Zip and lock with a password…" },
    Item { id: "add", label: "Add to archive…" },
    Item { id: "sfx", label: "Make a self-extracting .exe…" },
];

pub const ARCHIVE_ITEMS: &[Item] = &[
    Item { id: "extract-here", label: "Extract here" },
    Item { id: "open", label: "Open in Zipkoo" },
];

pub const ARCHIVE_EXTS: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "tgz", "zst", "tzst", "bz2", "tbz2", "xz", "txz", "001",
];

pub fn is_installed() -> bool {
    imp::is_installed()
}

pub fn install() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    imp::install(&exe)
}

pub fn uninstall() -> std::io::Result<()> {
    imp::uninstall()
}

pub fn supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::path::Path;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const ROOT: &str = r"Software\Classes";

    fn verb(classes: &RegKey, base: &str, title: &str, exe: &Path, items: &[Item]) -> std::io::Result<()> {
        let (k, _) = classes.create_subkey(base)?;
        k.set_value("MUIVerb", &title)?;
        k.set_value("Icon", &format!("\"{}\",0", exe.display()))?;
        k.set_value("SubCommands", &"")?;
        // Lets the verb show (and launch once per item) for large selections.
        k.set_value("MultiSelectModel", &"Player")?;
        for (i, it) in items.iter().enumerate() {
            let (sub, _) = k.create_subkey(format!(r"shell\{:02}{}", i + 1, it.id))?;
            sub.set_value("MUIVerb", &it.label)?;
            let (cmd, _) = sub.create_subkey("command")?;
            cmd.set_value("", &format!("\"{}\" --action {} \"%1\"", exe.display(), it.id))?;
        }
        Ok(())
    }

    pub fn install(exe: &Path) -> std::io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (classes, _) = hkcu.create_subkey(ROOT)?;
        verb(&classes, r"*\shell\Zipkoo", "Zipkoo", exe, FILE_ITEMS)?;
        verb(&classes, r"Directory\shell\Zipkoo", "Zipkoo", exe, FILE_ITEMS)?;
        for ext in ARCHIVE_EXTS {
            verb(&classes, &format!(r"SystemFileAssociations\.{ext}\shell\ZipkooExtract"), "Extract with Zipkoo", exe, ARCHIVE_ITEMS)?;
        }
        notify();
        Ok(())
    }

    pub fn uninstall() -> std::io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let classes = hkcu.open_subkey_with_flags(ROOT, winreg::enums::KEY_ALL_ACCESS)?;
        let _ = classes.delete_subkey_all(r"*\shell\Zipkoo");
        let _ = classes.delete_subkey_all(r"Directory\shell\Zipkoo");
        for ext in ARCHIVE_EXTS {
            let _ = classes.delete_subkey_all(format!(r"SystemFileAssociations\.{ext}\shell\ZipkooExtract"));
        }
        notify();
        Ok(())
    }

    pub fn is_installed() -> bool {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"{ROOT}\*\shell\Zipkoo")).is_ok()
    }

    /// Ask Explorer to refresh its menus.
    fn notify() {
        #[link(name = "shell32")]
        extern "system" {
            fn SHChangeNotify(event: i32, flags: u32, a: *const core::ffi::c_void, b: *const core::ffi::c_void);
        }
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, core::ptr::null(), core::ptr::null()) };
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    const MAC_ITEMS: &[(&str, &str, bool)] = &[
        ("zip-here", "Zip with Zipkoo", false),
        ("extract-here", "Extract with Zipkoo", true),
        ("add", "Add to Zipkoo archive…", false),
        ("open", "Open in Zipkoo", true),
    ];

    fn services_dir() -> std::io::Result<PathBuf> {
        let home = std::env::var_os("HOME").ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set"))?;
        Ok(PathBuf::from(home).join("Library/Services"))
    }

    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    fn info_plist(title: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>NSServices</key><array><dict>
<key>NSMenuItem</key><dict><key>default</key><string>{t}</string></dict>
<key>NSMessage</key><string>runWorkflowAsService</string>
<key>NSRequiredContext</key><dict><key>NSApplicationIdentifier</key><string>com.apple.finder</string></dict>
<key>NSSendFileTypes</key><array><string>public.item</string></array>
</dict></array>
</dict></plist>"#,
            t = esc(title)
        )
    }

    fn document_wflow(command: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>AMApplicationBuild</key><string>523</string>
<key>AMApplicationVersion</key><string>2.10</string>
<key>AMDocumentVersion</key><string>2</string>
<key>actions</key><array><dict><key>action</key><dict>
<key>AMAccepts</key><dict><key>Container</key><string>List</string><key>Optional</key><true/><key>Types</key><array><string>com.apple.cocoa.string</string></array></dict>
<key>AMActionVersion</key><string>2.0.3</string>
<key>AMApplication</key><array><string>Automator</string></array>
<key>AMParameterProperties</key><dict><key>COMMAND_STRING</key><dict/><key>CheckedForUserDefaultShell</key><dict/><key>inputMethod</key><dict/><key>shell</key><dict/><key>source</key><dict/></dict>
<key>AMProvides</key><dict><key>Container</key><string>List</string><key>Types</key><array><string>com.apple.cocoa.string</string></array></dict>
<key>ActionBundlePath</key><string>/System/Library/Automator/Run Shell Script.action</string>
<key>ActionName</key><string>Run Shell Script</string>
<key>ActionParameters</key><dict>
<key>COMMAND_STRING</key><string>{c}</string>
<key>CheckedForUserDefaultShell</key><true/>
<key>inputMethod</key><integer>1</integer>
<key>shell</key><string>/bin/zsh</string>
<key>source</key><string></string>
</dict>
<key>BundleIdentifier</key><string>com.apple.RunShellScript</string>
<key>CFBundleVersion</key><string>2.0.3</string>
<key>CanShowSelectedItemsWhenRun</key><false/>
<key>CanShowWhenRun</key><true/>
<key>Category</key><array><string>AMCategoryUtilities</string></array>
<key>Class Name</key><string>RunShellScriptAction</string>
<key>InputUUID</key><string>6E0D2C7C-3F0B-4C1E-9D6E-5A1B2C3D4E5F</string>
<key>Keywords</key><array><string>Shell</string></array>
<key>OutputUUID</key><string>7F1E3D8D-4A1C-4D2F-8E7F-6B2C3D4E5F60</string>
<key>UUID</key><string>8A2F4E9E-5B2D-4E30-9F80-7C3D4E5F6071</string>
<key>UnlocalizedApplications</key><array><string>Automator</string></array>
<key>arguments</key><dict/>
<key>isViewVisible</key><integer>1</integer>
<key>location</key><string>309.5:253.0</string>
<key>nibPath</key><string>/System/Library/Automator/Run Shell Script.action/Contents/Resources/Base.lproj/main.nib</string>
</dict><key>isViewVisible</key><integer>1</integer></dict></array>
<key>connectors</key><dict/>
<key>workflowMetaData</key><dict>
<key>serviceInputTypeIdentifier</key><string>com.apple.Automator.fileSystemObject</string>
<key>serviceOutputTypeIdentifier</key><string>com.apple.Automator.nothing</string>
<key>serviceProcessesInput</key><integer>0</integer>
<key>workflowTypeIdentifier</key><string>com.apple.Automator.servicesMenu</string>
</dict>
</dict></plist>"#,
            c = esc(command)
        )
    }

    fn workflow_path(dir: &Path, title: &str) -> PathBuf {
        dir.join(format!("{title}.workflow"))
    }

    pub fn install(exe: &Path) -> std::io::Result<()> {
        let dir = services_dir()?;
        for (id, title, _archives) in MAC_ITEMS {
            let wf = workflow_path(&dir, title).join("Contents");
            fs::create_dir_all(&wf)?;
            fs::write(wf.join("Info.plist"), info_plist(title))?;
            let quoted = exe.display().to_string().replace('\'', r"'\''");
            let cmd = format!("'{quoted}' --action {id} \"$@\" >/dev/null 2>&1 &");
            fs::write(wf.join("document.wflow"), document_wflow(&cmd))?;
        }
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").status();
        Ok(())
    }

    pub fn uninstall() -> std::io::Result<()> {
        let dir = services_dir()?;
        for (_, title, _) in MAC_ITEMS {
            let _ = fs::remove_dir_all(workflow_path(&dir, title));
        }
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").status();
        Ok(())
    }

    pub fn is_installed() -> bool {
        services_dir().map(|d| workflow_path(&d, MAC_ITEMS[0].1).exists()).unwrap_or(false)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::path::Path;
    pub fn install(_exe: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "The right-click menu is available on Windows and macOS."))
    }
    pub fn uninstall() -> std::io::Result<()> {
        Ok(())
    }
    pub fn is_installed() -> bool {
        false
    }
}

/// One menu click, possibly many files.
#[derive(Clone, Debug, serde::Serialize, PartialEq)]
pub struct ShellAction {
    pub action: String,
    pub paths: Vec<String>,
}

/// Turn a command line into an action. `Zipkoo a.zip` (file association) becomes `open`;
/// `Zipkoo x.jpg y.txt` becomes `add`.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Option<ShellAction> {
    let mut action: Option<String> = None;
    let mut paths = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--action" {
            action = it.next();
        } else if a.starts_with("--") || a.starts_with("-psn_") {
            continue;
        } else {
            paths.push(a);
        }
    }
    if paths.is_empty() {
        return None;
    }
    let action = action.unwrap_or_else(|| {
        let archive = paths.len() == 1 && {
            let lower = paths[0].to_ascii_lowercase();
            ARCHIVE_EXTS.iter().any(|e| lower.ends_with(&format!(".{e}")))
        };
        if archive { "open".into() } else { "add".into() }
    });
    Some(ShellAction { action, paths })
}

/// Collects clicks that arrive as separate launches (Explorer starts one process per selected
/// item) and merges those with the same action that arrive within a short window.
#[derive(Default)]
pub struct Inbox {
    queue: std::sync::Mutex<Vec<(ShellAction, std::time::Instant)>>,
}

pub const MERGE_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

impl Inbox {
    pub fn push(&self, a: ShellAction) {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        if let Some((last, at)) = q.last_mut() {
            if last.action == a.action && a.action != "open" && now.duration_since(*at) < MERGE_WINDOW {
                for p in a.paths {
                    if !last.paths.contains(&p) {
                        last.paths.push(p);
                    }
                }
                *at = now;
                return;
            }
        }
        q.push((a, now));
    }

    /// Actions whose merge window has closed.
    pub fn take_ready(&self) -> Vec<ShellAction> {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        let (ready, wait): (Vec<_>, Vec<_>) = q.drain(..).partition(|(_, at)| now.duration_since(*at) >= MERGE_WINDOW);
        *q = wait;
        ready.into_iter().map(|(a, _)| a).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_menu_and_association_launches() {
        assert_eq!(parse_args(s(&["--action", "zip-here", "C:\\a.txt"])).unwrap(), ShellAction { action: "zip-here".into(), paths: s(&["C:\\a.txt"]) });
        assert_eq!(parse_args(s(&["/x/Photos.7z"])).unwrap().action, "open");
        assert_eq!(parse_args(s(&["/x/a.jpg", "/x/b.jpg"])).unwrap().action, "add");
        assert_eq!(parse_args(s(&["-psn_0_123"])), None);
        assert_eq!(parse_args(s(&[])), None);
    }

    #[test]
    fn merges_quick_clicks() {
        let inbox = Inbox::default();
        inbox.push(ShellAction { action: "zip-here".into(), paths: s(&["a"]) });
        inbox.push(ShellAction { action: "zip-here".into(), paths: s(&["b", "a"]) });
        inbox.push(ShellAction { action: "extract-here".into(), paths: s(&["c.zip"]) });
        assert!(inbox.take_ready().is_empty());
        std::thread::sleep(MERGE_WINDOW + std::time::Duration::from_millis(50));
        let ready = inbox.take_ready();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].paths, s(&["a", "b"]));
        assert_eq!(ready[1].action, "extract-here");
    }
}
