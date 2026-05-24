use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SOURCE_FETCH_REPORT_DEFAULT: &str = "target/source-fetch-provenance/SOURCE_FETCH.txt";
const LIBGHOSTTY_VT_SOURCE: &str = r#"source = "git+https://github.com/uzaaft/libghostty-rs.git?rev=31d1f70004ff80727e36437cd540984f927333ce#31d1f70004ff80727e36437cd540984f927333ce""#;

fn main() {
    if let Err(err) = run() {
        eprintln!("xtask: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        return usage_error();
    };
    if args.next().is_some() {
        return usage_error();
    }

    match command.as_str() {
        "static-link-verify" => static_link_verify(),
        "source-fetch-provenance-verify" => source_fetch_provenance_verify(),
        _ => usage_error(),
    }
}

fn usage_error() -> Result<()> {
    Err("usage: xtask <static-link-verify|source-fetch-provenance-verify>".into())
}

fn static_link_verify() -> Result<()> {
    println!("building libghostty-vt release binary for static-link verification");
    let target_dir = Path::new("target/static-link-verify");
    let status = Command::new("cargo")
        .args(["build", "-p", "nmux-cli", "--release", "--bin", "nmux"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("CARGO_TARGET_DIR", target_dir)
        .status()?;
    if !status.success() {
        return Err(format!("cargo release build failed with status {status}").into());
    }

    let bin = target_dir.join("release/nmux");
    require_file(&bin, "release nmux binary")?;
    let deps = target_dir.join("DYNAMIC_DEPS.txt");
    write_dynamic_dependency_report(&bin, &deps)?;
    let dep_report = fs::read_to_string(&deps)?;
    if contains_case_insensitive(&dep_report, "libghostty-vt")
        || contains_case_insensitive(&dep_report, "ghostty-vt")
    {
        return Err(
            "release nmux dynamically depends on libghostty-vt; static linking is required".into(),
        );
    }

    println!("static_link_verified={}", bin.display());
    println!("dynamic_dependency_report={}", deps.display());
    Ok(())
}

fn write_dynamic_dependency_report(bin: &Path, deps: &Path) -> Result<()> {
    if command_exists("otool") {
        write_command_output(deps, Command::new("otool").arg("-L").arg(bin))
    } else if command_exists("ldd") {
        write_command_output(deps, Command::new("ldd").arg(bin))
    } else {
        Err("dynamic dependency inspector unavailable".into())
    }
}

fn write_command_output(path: &Path, command: &mut Command) -> Result<()> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("dependency inspector failed with status {}", output.status).into());
    }
    fs::write(path, output.stdout)?;
    Ok(())
}

fn command_exists(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file()
    })
}

fn source_fetch_provenance_verify() -> Result<()> {
    println!("verifying source-fetch provenance report");
    let report =
        env::var("SOURCE_FETCH_REPORT").unwrap_or_else(|_| SOURCE_FETCH_REPORT_DEFAULT.into());
    let report_path = PathBuf::from(report);
    let report_text = read_required_text(&report_path, "source-fetch provenance artifact")?;

    require_exact(
        &report_text,
        "nmux source-fetch provenance sample",
        "report title",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| {
            line.strip_prefix("generated_at_utc=")
                .is_some_and(is_utc_timestamp)
        },
        "generation timestamp",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| {
            matches!(
                line,
                "ghostty_source_mode=pinned-fetch" | "ghostty_source_mode=local"
            )
        },
        "Ghostty source mode",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR field",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| field_has_value(line, "GIT_CONFIG_GLOBAL="),
        "GIT_CONFIG_GLOBAL field",
        &report_path,
    )?;
    require_exact(
        &report_text,
        &format!(
            "Cargo.lock sha256={}",
            sha256_file(Path::new("Cargo.lock"))?
        ),
        "Cargo.lock hash",
        &report_path,
    )?;
    require_exact(
        &report_text,
        "[toolchain]",
        "toolchain section",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| line.starts_with("cargo=cargo "),
        "cargo version",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| line.starts_with("rustc=rustc "),
        "rustc version",
        &report_path,
    )?;
    require_exact(
        &report_text,
        "flatc=flatc version 25.12.19",
        "flatc version",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| line.starts_with("zig=0.15."),
        "Zig 0.15 version",
        &report_path,
    )?;
    require_line_where(
        &report_text,
        |line| {
            matches!(
                line,
                "ghostty_source_dir_status=unset"
                    | "ghostty_source_dir_status=present"
                    | "ghostty_source_dir_status=missing"
            )
        },
        "Ghostty source dir status",
        &report_path,
    )?;

    require_exact(
        &report_text,
        "[cargo_lock:libghostty-vt]",
        "libghostty-vt section",
        &report_path,
    )?;
    require_exact(
        &report_text,
        r#"name = "libghostty-vt""#,
        "libghostty-vt package name",
        &report_path,
    )?;
    require_exact(
        &report_text,
        LIBGHOSTTY_VT_SOURCE,
        "libghostty-vt pinned revision",
        &report_path,
    )?;
    require_lock_section(&report_text, "libghostty-vt", &report_path)?;

    require_exact(
        &report_text,
        "[cargo_lock:libghostty-vt-sys]",
        "libghostty-vt-sys section",
        &report_path,
    )?;
    require_exact(
        &report_text,
        r#"name = "libghostty-vt-sys""#,
        "libghostty-vt-sys package name",
        &report_path,
    )?;
    require_lock_section(&report_text, "libghostty-vt-sys", &report_path)?;

    require_exact(
        &report_text,
        "[policy_note]",
        "policy note section",
        &report_path,
    )?;
    require_exact(
        &report_text,
        "This report records local source-fetch inputs for evidence. It does not choose the default or packaged-build source policy.",
        "policy note",
        &report_path,
    )?;
    println!("source_fetch_provenance_verified={}", report_path.display());
    Ok(())
}

fn require_lock_section(report_text: &str, package: &str, report_path: &Path) -> Result<()> {
    let lock_record = cargo_lock_record(package)?;
    let section = report_section(report_text, &format!("[cargo_lock:{package}]"));
    for line in lock_record.lines().filter(|line| !line.is_empty()) {
        if !section.lines().any(|section_line| section_line == line) {
            return Err(format!(
                "missing source-fetch provenance Cargo.lock line for {package}: {line}"
            )
            .into());
        }
    }
    if section.is_empty() {
        return Err(format!(
            "missing source-fetch provenance record in {}: cargo lock section for {package}",
            report_path.display()
        )
        .into());
    }
    Ok(())
}

fn cargo_lock_record(package: &str) -> Result<String> {
    let lock = fs::read_to_string("Cargo.lock")?;
    for block in lock.split("\n\n") {
        if block
            .lines()
            .any(|line| line.trim() == format!(r#"name = "{package}""#))
        {
            return Ok(format!("{block}\n"));
        }
    }
    Err(format!("missing Cargo.lock record for {package}").into())
}

fn report_section<'a>(report_text: &'a str, header: &str) -> &'a str {
    let Some(start) = report_text.find(header) else {
        return "";
    };
    let after_header = &report_text[start + header.len()..];
    let after_newline = after_header.strip_prefix('\n').unwrap_or(after_header);
    let end = after_newline.find("\n[").unwrap_or(after_newline.len());
    &after_newline[..end]
}

fn read_required_text(path: &Path, description: &str) -> Result<String> {
    require_file(path, description)?;
    Ok(fs::read_to_string(path)?)
}

fn require_file(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::metadata(path)
        .map_err(|_| format!("missing or empty {description}: {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!("missing or empty {description}: {}", path.display()).into());
    }
    Ok(())
}

fn require_exact(text: &str, line: &str, description: &str, path: &Path) -> Result<()> {
    if text.lines().any(|candidate| candidate == line) {
        return Ok(());
    }
    Err(format!(
        "missing source-fetch provenance record in {}: {description}",
        path.display()
    )
    .into())
}

fn require_line_where(
    text: &str,
    predicate: impl Fn(&str) -> bool,
    description: &str,
    path: &Path,
) -> Result<()> {
    if text.lines().any(predicate) {
        return Ok(());
    }
    Err(format!(
        "missing source-fetch provenance record in {}: {description}",
        path.display()
    )
    .into())
}

fn sha256_file(path: &Path) -> Result<String> {
    let output = if command_exists("sha256sum") {
        Command::new("sha256sum").arg(path).output()?
    } else if command_exists("shasum") {
        Command::new("shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()?
    } else {
        return Err("missing sha256sum or shasum for provenance verification".into());
    };
    if !output.status.success() {
        return Err(format!("hash command failed with status {}", output.status).into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    stdout
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| "hash command did not print a SHA-256 digest".into())
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn field_has_value(line: &str, prefix: &str) -> bool {
    line.strip_prefix(prefix)
        .is_some_and(|value| !value.is_empty())
}

fn is_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == "YYYY-MM-DDTHH:MM:SSZ".len()
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
            .all(|(_, byte)| byte.is_ascii_digit())
}
