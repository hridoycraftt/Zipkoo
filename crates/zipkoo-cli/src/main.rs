//! `zipkoo` command line: pack, open, peek, list and check archives.
use clap::{Parser, Subcommand, ValueEnum};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use zipkoo_core::{extract, inspect, pack, peek, ExtractOptions, Format, Level, PackOptions, Progress};

#[derive(Parser)]
#[command(name = "zipkoo", version, about = "Zip it. Ship it. Friendly archives from the command line.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Copy, Clone, ValueEnum)]
enum Fmt { Zip, #[value(name = "7z")] SevenZ, #[value(name = "tar.zst")] TarZst }

#[derive(Copy, Clone, ValueEnum)]
enum Lvl { Fast, Normal, Max }

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
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", units[i]) }
}

fn bar() -> impl FnMut(&Progress) {
    let tty = std::io::stderr().is_terminal();
    move |p: &Progress| {
        if !tty || p.total_bytes == 0 {
            return;
        }
        let pct = (p.done_bytes as f64 / p.total_bytes as f64 * 100.0).min(100.0);
        let filled = (pct / 5.0) as usize;
        let name: String = p.current.chars().rev().take(40).collect::<String>().chars().rev().collect();
        eprint!("\r\x1b[2K[{}{}] {pct:>5.1}%  {name}", "█".repeat(filled), "░".repeat(20 - filled));
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
    std::io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Pack { inputs, out, format, level, lock, hide_names, no_smart, split } => {
            if inputs.is_empty() {
                return Err("Add at least one file or folder to pack.".into());
            }
            let format = match format { Fmt::Zip => Format::Zip, Fmt::SevenZ => Format::SevenZ, Fmt::TarZst => Format::TarZst };
            let level = match level { Lvl::Fast => Level::Fast, Lvl::Normal => Level::Normal, Lvl::Max => Level::Max };
            let split_size = match split {
                Some(s) => Some(parse_size(&s).ok_or(format!("Can't read the split size \"{s}\". Try 700M or 2G."))?),
                None => None,
            };
            let out = out.unwrap_or_else(|| {
                let stem = inputs[0].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
                PathBuf::from(format!("{stem}.{}", format.extension()))
            });
            let password = if lock { Some(ask_password(true)?) } else { None };
            let opts = PackOptions { format, level, smart: !no_smart, password, hide_names, split_size, ..Default::default() };
            let start = std::time::Instant::now();
            let r = pack(&inputs, &out, &opts, &mut bar(), None).map_err(|e| e.to_string())?;
            eprintln!();
            let saved = 100.0 - (r.output_bytes as f64 / r.input_bytes.max(1) as f64 * 100.0);
            println!("✓ {} files · {} → {} (saved {:.0}%) · {:.1}s", r.files, human(r.input_bytes), human(r.output_bytes), saved.max(0.0), start.elapsed().as_secs_f64());
            if r.stored_as_is > 0 {
                println!("  {} already-compressed files stored as they are", r.stored_as_is);
            }
            for o in r.outputs {
                println!("  {}", o.display());
            }
        }
        Cmd::Open { archive, to, overwrite, no_programs, password } => {
            let info = inspect(&archive, password.as_deref()).map_err(|e| e.to_string())?;
            let password = match password {
                None if info.encrypted => Some(ask_password(false)?),
                p => p,
            };
            if !info.executables.is_empty() && !no_programs {
                eprintln!("⚠ {} program(s) inside. Open them only if you trust the sender.", info.executables.len());
            }
            let dest = to.unwrap_or_else(|| {
                let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let name = zipkoo_core::split::strip_part_suffix(&name).to_string();
                let stem = name.trim_end_matches(&format!(".{}", info.format.extension())).to_string();
                archive.with_file_name(stem)
            });
            let opts = ExtractOptions { password: password.as_deref(), overwrite, allow_executables: !no_programs, ..Default::default() };
            let r = extract(&archive, &dest, &opts, &mut bar(), None).map_err(|e| e.to_string())?;
            eprintln!();
            println!("✓ {} files · {} → {}", r.files, human(r.bytes), r.dest.display());
            for (name, why) in r.skipped {
                println!("  skipped {name} ({why})");
            }
        }
        Cmd::Peek { archive, file, password } => match file {
            Some(name) => {
                let data = peek(&archive, &name, password.as_deref(), 256 << 20).map_err(|e| e.to_string())?;
                std::io::stdout().write_all(&data).map_err(|e| e.to_string())?;
            }
            None => {
                let i = inspect(&archive, password.as_deref()).map_err(|e| e.to_string())?;
                println!("{} · {} · {} files · {} packed into {}{}", archive.display(), i.format.label(), i.file_count, human(i.total_size), human(i.archive_size), if i.encrypted { " · 🔒 encrypted" } else { "" });
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
        Cmd::Hash { file } => {
            println!("{}  {}", zipkoo_core::hash::sha256_file(&file).map_err(|e| e.to_string())?, file.display());
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
