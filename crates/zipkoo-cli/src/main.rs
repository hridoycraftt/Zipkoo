//! `zipkoo` command line: pack, open, peek, list and check archives.
use clap::{Parser, Subcommand, ValueEnum};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use zipkoo_core::{
    extract, inspect, pack, peek, ExtractOptions, Format, Level, PackOptions, Progress,
};

#[derive(Parser)]
#[command(
    name = "zipkoo",
    version,
    about = "Zip it. Ship it. Friendly archives from the command line."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Copy, Clone, ValueEnum)]
enum Fmt {
    Zip,
    #[value(name = "7z")]
    SevenZ,
    #[value(name = "tar.zst")]
    TarZst,
}

#[derive(Copy, Clone, ValueEnum)]
enum Lvl {
    Fast,
    Normal,
    Max,
}

#[derive(Subcommand)]
enum Cmd {
    /// Pack files and folders into an archive
    Pack {
        inputs: Vec<PathBuf>,
        /// Output file. Defaults to the first input's name plus the format extension.
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(short, long, value_enum, default_value = "zip")]
        format: Fmt,
        #[arg(short, long, value_enum, default_value = "normal")]
        level: Lvl,
        /// Ask for a password and encrypt with AES-256
        #[arg(long)]
        lock: bool,
        /// 7z only: hide file names too
        #[arg(long)]
        hide_names: bool,
        /// Compress everything, even files that are already compressed
        #[arg(long)]
        no_smart: bool,
        /// Split into parts, e.g. 700M, 2G
        #[arg(long)]
        split: Option<String>,
    },
    /// Extract an archive (alias: x)
    #[command(alias = "x")]
    Open {
        archive: PathBuf,
        /// Destination folder. Defaults to a folder named after the archive.
        #[arg(short, long)]
        to: Option<PathBuf>,
        #[arg(long)]
        overwrite: bool,
        /// Skip files that can run code (.exe, .bat, .app ...)
        #[arg(long)]
        no_programs: bool,
        #[arg(short, long)]
        password: Option<String>,
    },
    /// List what is inside (alias: ls)
    #[command(alias = "ls")]
    Peek {
        archive: PathBuf,
        /// Print one file's contents instead of the list
        file: Option<String>,
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Make a self-extracting program (needs the zipkoo-sfx stub next to this binary, or --stub)
    Sfx {
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Ask for a password and encrypt with AES-256
        #[arg(long)]
        lock: bool,
        #[arg(long)]
        stub: Option<PathBuf>,
    },
    /// Share a file with an end-to-end encrypted link
    Share {
        file: PathBuf,
        /// Share server, e.g. https://share.example.com
        #[arg(long, env = "ZIPKOO_SHARE_SERVER")]
        server: String,
        /// Hours until the link stops working
        #[arg(long, default_value_t = 24)]
        hours: u32,
        /// Stop after this many downloads
        #[arg(long)]
        downloads: Option<u32>,
        /// Also require a password
        #[arg(long)]
        lock: bool,
    },
    /// Download a share link
    Get {
        link: String,
        #[arg(short, long, default_value = ".")]
        to: PathBuf,
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Smart Update: add only new and changed files to a ZIP (keeps history)
    Update {
        archive: PathBuf,
        inputs: Vec<PathBuf>,
        /// Remove files that are no longer in the inputs
        #[arg(long)]
        remove_missing: bool,
        #[arg(long)]
        no_history: bool,
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Show an archive's version history
    History { archive: PathBuf },
    /// Self-heal: add repair data, check, or repair an archive
    Heal {
        archive: PathBuf,
        /// Create a repair file with this much redundancy (percent)
        #[arg(long)]
        create: Option<u8>,
        /// Repair damaged blocks
        #[arg(long)]
        repair: bool,
    },
    /// Convert archives to another format (works on many files)
    Convert {
        archives: Vec<PathBuf>,
        #[arg(short, long, value_enum, default_value = "7z")]
        format: Fmt,
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Snapshot a folder into a backup folder (skips if nothing changed)
    Backup {
        folder: PathBuf,
        dest: PathBuf,
        #[arg(long, default_value_t = 10)]
        keep: usize,
        #[arg(long)]
        repair: Option<u8>,
    },
    /// Search file names inside every archive under a folder
    Search {
        query: String,
        #[arg(long, default_value = ".")]
        r#in: PathBuf,
    },
    /// Find identical files
    Dupes { inputs: Vec<PathBuf> },
    /// Shrink photos into a copy, ready to pack
    Shrink {
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 2560)]
        max_edge: u32,
        #[arg(long, default_value_t = 82)]
        quality: u8,
    },
    /// Print the SHA-256 of a file
    Hash { file: PathBuf },
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_uppercase();
    let (num, mul) = match s.chars().last()? {
        'K' => (&s[..s.len() - 1], 1u64 << 10),
        'M' => (&s[..s.len() - 1], 1 << 20),
        'G' => (&s[..s.len() - 1], 1 << 30),
        _ => (&s[..], 1),
    };
    num.parse::<f64>().ok().map(|n| (n * mul as f64) as u64)
}

fn human(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

fn bar() -> impl FnMut(&Progress) {
    let tty = std::io::stderr().is_terminal();
    move |p: &Progress| {
        if !tty || p.total_bytes == 0 {
            return;
        }
        let pct = (p.done_bytes as f64 / p.total_bytes as f64 * 100.0).min(100.0);
        let filled = (pct / 5.0) as usize;
        let name: String = p
            .current
            .chars()
            .rev()
            .take(40)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        eprint!(
            "\r\x1b[2K[{}{}] {pct:>5.1}%  {name}",
            "█".repeat(filled),
            "░".repeat(20 - filled)
        );
        let _ = std::io::stderr().flush();
    }
}

fn ask_password(confirm: bool) -> Result<String, String> {
    let pw = rpassword_like("Password: ")?;
    if confirm && rpassword_like("Repeat password: ")? != pw {
        return Err("The passwords don't match.".into());
    }
    if pw.is_empty() {
        return Err("The password can't be empty.".into());
    }
    Ok(pw)
}

/// Minimal prompt. Echo is not hidden; the desktop app has a proper password field.
fn rpassword_like(prompt: &str) -> Result<String, String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut s = String::new();
    std::io::stdin()
        .read_line(&mut s)
        .map_err(|e| e.to_string())?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Pack {
            inputs,
            out,
            format,
            level,
            lock,
            hide_names,
            no_smart,
            split,
        } => {
            if inputs.is_empty() {
                return Err("Add at least one file or folder to pack.".into());
            }
            let format = match format {
                Fmt::Zip => Format::Zip,
                Fmt::SevenZ => Format::SevenZ,
                Fmt::TarZst => Format::TarZst,
            };
            let level = match level {
                Lvl::Fast => Level::Fast,
                Lvl::Normal => Level::Normal,
                Lvl::Max => Level::Max,
            };
            let split_size = match split {
                Some(s) => Some(parse_size(&s).ok_or(format!(
                    "Can't read the split size \"{s}\". Try 700M or 2G."
                ))?),
                None => None,
            };
            let out = out.unwrap_or_else(|| {
                let stem = inputs[0]
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "archive".into());
                zipkoo_core::unique_path(&PathBuf::from(format!("{stem}.{}", format.extension())))
            });
            let password = if lock {
                Some(ask_password(true)?)
            } else {
                None
            };
            let opts = PackOptions {
                format,
                level,
                smart: !no_smart,
                password,
                hide_names,
                split_size,
                ..Default::default()
            };
            let start = std::time::Instant::now();
            let r = pack(&inputs, &out, &opts, &mut bar(), None).map_err(|e| e.to_string())?;
            eprintln!();
            let saved = 100.0 - (r.output_bytes as f64 / r.input_bytes.max(1) as f64 * 100.0);
            println!(
                "✓ {} files · {} → {} (saved {:.0}%) · {:.1}s",
                r.files,
                human(r.input_bytes),
                human(r.output_bytes),
                saved.max(0.0),
                start.elapsed().as_secs_f64()
            );
            if r.stored_as_is > 0 {
                println!(
                    "  {} already-compressed files stored as they are",
                    r.stored_as_is
                );
            }
            for o in r.outputs {
                println!("  {}", o.display());
            }
        }
        Cmd::Open {
            archive,
            to,
            overwrite,
            no_programs,
            password,
        } => {
            let info = inspect(&archive, password.as_deref()).map_err(|e| e.to_string())?;
            let password = match password {
                None if info.encrypted => Some(ask_password(false)?),
                p => p,
            };
            if !info.executables.is_empty() && !no_programs {
                eprintln!(
                    "⚠ {} program(s) inside. Open them only if you trust the sender.",
                    info.executables.len()
                );
            }
            let dest = to.unwrap_or_else(|| {
                let name = archive
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = zipkoo_core::split::strip_part_suffix(&name).to_string();
                let stem = name
                    .trim_end_matches(&format!(".{}", info.format.extension()))
                    .to_string();
                archive.with_file_name(stem)
            });
            let opts = ExtractOptions {
                password: password.as_deref(),
                overwrite,
                allow_executables: !no_programs,
                ..Default::default()
            };
            let r = extract(&archive, &dest, &opts, &mut bar(), None).map_err(|e| e.to_string())?;
            eprintln!();
            println!(
                "✓ {} files · {} → {}",
                r.files,
                human(r.bytes),
                r.dest.display()
            );
            for (name, why) in r.skipped {
                println!("  skipped {name} ({why})");
            }
        }
        Cmd::Peek {
            archive,
            file,
            password,
        } => match file {
            Some(name) => {
                let data = peek(&archive, &name, password.as_deref(), 256 << 20)
                    .map_err(|e| e.to_string())?;
                std::io::stdout()
                    .write_all(&data)
                    .map_err(|e| e.to_string())?;
            }
            None => {
                let i = inspect(&archive, password.as_deref()).map_err(|e| e.to_string())?;
                println!(
                    "{} · {} · {} files · {} packed into {}{}",
                    archive.display(),
                    i.format.label(),
                    i.file_count,
                    human(i.total_size),
                    human(i.archive_size),
                    if i.encrypted {
                        " · 🔒 encrypted"
                    } else {
                        ""
                    }
                );
                for e in &i.entries {
                    let flag = if e.executable { " ⚠ program" } else { "" };
                    if e.is_dir {
                        println!("  {:>10}  {}/", "", e.name.trim_end_matches('/'));
                    } else {
                        println!("  {:>10}  {}{flag}", human(e.size), e.name);
                    }
                }
            }
        },
        Cmd::Sfx {
            inputs,
            out,
            lock,
            stub,
        } => {
            if inputs.is_empty() {
                return Err("Add at least one file or folder.".into());
            }
            let name = if cfg!(windows) {
                "zipkoo-sfx.exe"
            } else {
                "zipkoo-sfx"
            };
            let stub = stub
                .or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join(name))))
                .filter(|p| p.is_file())
                .ok_or("Can't find the zipkoo-sfx stub. Build it with `cargo build -p zipkoo-sfx` or pass --stub.")?;
            let out = out.unwrap_or_else(|| {
                let stem = inputs[0]
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Archive".into());
                zipkoo_core::unique_path(&PathBuf::from(if cfg!(windows) {
                    format!("{stem}.exe")
                } else {
                    format!("{stem}.run")
                }))
            });
            let password = if lock {
                Some(ask_password(true)?)
            } else {
                None
            };
            let opts = PackOptions {
                password,
                ..Default::default()
            };
            let r = zipkoo_core::sfx::build(&stub, &inputs, &out, &opts, &mut bar(), None)
                .map_err(|e| e.to_string())?;
            eprintln!();
            println!(
                "✓ {} files · {} → {} ({})",
                r.files,
                human(r.input_bytes),
                out.display(),
                human(r.output_bytes)
            );
        }
        Cmd::Share {
            file,
            server,
            hours,
            downloads,
            lock,
        } => {
            let pw = if lock {
                Some(ask_password(true)?)
            } else {
                None
            };
            let opts = zipkoo_core::share::ShareOptions {
                server: &server,
                expires_hours: hours,
                max_downloads: downloads,
                password: pw.as_deref(),
            };
            let s =
                zipkoo_core::share::upload(&file, &opts, &mut |_| {}).map_err(|e| e.to_string())?;
            println!("{}", s.url);
            eprintln!(
                "Delete token (keep it to remove the link early): {}",
                s.delete_token
            );
        }
        Cmd::Get { link, to, password } => {
            let p = zipkoo_core::share::download(&link, &to, password.as_deref(), &mut |_| {})
                .map_err(|e| e.to_string())?;
            println!("✓ {}", p.display());
        }
        Cmd::Update {
            archive,
            inputs,
            remove_missing,
            no_history,
            password,
        } => {
            let opts = zipkoo_core::update::UpdateOptions {
                password: password.as_deref(),
                keep_history: !no_history,
                remove_missing,
                smart: true,
            };
            let r = zipkoo_core::update::update(&archive, &inputs, &opts, &mut bar(), None)
                .map_err(|e| e.to_string())?;
            eprintln!();
            println!(
                "✓ version {} · {} added · {} changed · {} removed · {} unchanged · packed {}",
                r.version,
                r.added,
                r.changed,
                r.removed,
                r.unchanged,
                human(r.bytes_packed)
            );
        }
        Cmd::History { archive } => {
            let m = zipkoo_core::update::read_manifest(&archive).map_err(|e| e.to_string())?;
            if m.versions.is_empty() {
                println!("No history yet. Run `zipkoo update` to start one.");
            }
            for v in m.versions {
                println!(
                    "v{}  {}  +{} ~{} -{}",
                    v.version,
                    v.time,
                    v.added.len(),
                    v.changed.len(),
                    v.removed.len()
                );
            }
        }
        Cmd::Heal {
            archive,
            create,
            repair,
        } => {
            if let Some(p) = create {
                let r = zipkoo_core::heal::create(&archive, p, &mut bar(), None)
                    .map_err(|e| e.to_string())?;
                eprintln!();
                println!("✓ repair data {} ({})", r.path.display(), human(r.size));
            } else {
                let h = zipkoo_core::heal::check(&archive, repair, &mut bar(), None)
                    .map_err(|e| e.to_string())?;
                eprintln!();
                if h.healthy() {
                    println!("✓ Healthy · {} blocks checked", h.blocks);
                } else if repair && h.repairable() {
                    println!("✓ Repaired {} damaged blocks", h.repaired);
                } else if h.repairable() {
                    println!(
                        "⚠ {} damaged blocks. Run with --repair to fix them.",
                        h.damaged
                    );
                } else {
                    return Err(format!(
                        "{} damaged blocks; {} are beyond the repair data.",
                        h.damaged, h.unrecoverable
                    ));
                }
            }
        }
        Cmd::Convert {
            archives,
            format,
            password,
        } => {
            let format = match format {
                Fmt::Zip => Format::Zip,
                Fmt::SevenZ => Format::SevenZ,
                Fmt::TarZst => Format::TarZst,
            };
            for a in archives {
                let out = zipkoo_core::convert::converted_path(&a, format);
                let opts = zipkoo_core::convert::ConvertOptions {
                    password: password.as_deref(),
                    pack: PackOptions {
                        format,
                        ..Default::default()
                    },
                };
                match zipkoo_core::convert::convert(&a, &out, &opts, &mut bar(), None) {
                    Ok(r) => {
                        eprintln!();
                        println!(
                            "✓ {} → {} ({})",
                            a.display(),
                            out.display(),
                            human(r.output_bytes)
                        );
                    }
                    Err(e) => {
                        eprintln!();
                        println!("✗ {}: {e}", a.display());
                    }
                }
            }
        }
        Cmd::Backup {
            folder,
            dest,
            keep,
            repair,
        } => {
            let opts = zipkoo_core::backup::BackupOptions {
                keep,
                recovery_percent: repair,
                ..Default::default()
            };
            match zipkoo_core::backup::snapshot(&folder, &dest, &opts, &mut bar(), None)
                .map_err(|e| e.to_string())?
            {
                Some(s) => {
                    eprintln!();
                    println!(
                        "✓ {} ({} files){}",
                        s.path.display(),
                        s.report.files,
                        if s.pruned.is_empty() {
                            String::new()
                        } else {
                            format!(" · removed {} old", s.pruned.len())
                        }
                    );
                }
                None => println!("Nothing changed since the last snapshot."),
            }
        }
        Cmd::Search { query, r#in } => {
            let db = std::env::temp_dir().join("zipkoo-cli-index.db");
            let mut ix = zipkoo_core::index::Index::open(&db).map_err(|e| e.to_string())?;
            ix.refresh(&zipkoo_core::index::find_archives(&r#in, 12))
                .map_err(|e| e.to_string())?;
            for h in ix.search(&query, 200).map_err(|e| e.to_string())? {
                println!("{:>10}  {}  ⟵  {}", human(h.size), h.name, h.archive);
            }
        }
        Cmd::Dupes { inputs } => {
            let g = zipkoo_core::dupes::find(&inputs, 1).map_err(|e| e.to_string())?;
            let wasted: u64 = g.iter().map(|g| g.wasted).sum();
            for grp in &g {
                println!("{} × {}", grp.files.len(), human(grp.size));
                for f in &grp.files {
                    println!("    {}", f.display());
                }
            }
            println!("{} groups · {} could be saved", g.len(), human(wasted));
        }
        Cmd::Shrink {
            inputs,
            out,
            max_edge,
            quality,
        } => {
            let r = zipkoo_core::optimize::optimize(
                &inputs,
                &out,
                &zipkoo_core::optimize::OptimizeOptions { max_edge, quality },
            )
            .map_err(|e| e.to_string())?;
            println!(
                "✓ {} photos ({} smaller) · {} → {} · {}",
                r.images,
                r.shrunk,
                human(r.before),
                human(r.after),
                out.display()
            );
        }
        Cmd::Hash { file } => {
            println!(
                "{}  {}",
                zipkoo_core::hash::sha256_file(&file).map_err(|e| e.to_string())?,
                file.display()
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zipkoo: {e}");
            ExitCode::FAILURE
        }
    }
}
