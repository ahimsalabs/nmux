use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SOURCE_FETCH_REPORT_DEFAULT: &str = "target/source-fetch-provenance/SOURCE_FETCH.txt";
const PACKAGING_PROVENANCE_MANIFEST_DEFAULT: &str =
    "target/packaging-libghostty-vt/package/PROVENANCE.txt";
const PACKAGING_LAYOUT_DEFAULT: &str = "target/packaging-libghostty-vt/package";
const PACKAGING_ARCHIVE_DEFAULT: &str =
    "target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz";
const LIBGHOSTTY_VT_SOURCE: &str = r#"source = "git+https://github.com/uzaaft/libghostty-rs.git?rev=31d1f70004ff80727e36437cd540984f927333ce#31d1f70004ff80727e36437cd540984f927333ce""#;
const RUNTIME_LIBRARY_STRATEGY: &str = "runtime_library_strategy=staged libghostty-vt native library artifacts for packaging evidence; nmux must not dynamically depend on libghostty-vt";

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
        "packaging-archive-verify" => packaging_archive_verify(),
        "packaging-provenance-manifest-verify" => packaging_provenance_manifest_verify(),
        "static-link-verify" => static_link_verify(),
        "source-fetch-provenance-verify" => source_fetch_provenance_verify(),
        _ => usage_error(),
    }
}

fn usage_error() -> Result<()> {
    Err("usage: xtask <packaging-archive-verify|packaging-provenance-manifest-verify|static-link-verify|source-fetch-provenance-verify>".into())
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

fn packaging_provenance_manifest_verify() -> Result<()> {
    println!("verifying opt-in libghostty-vt package provenance manifest");
    let manifest = env::var("PACKAGING_PROVENANCE_MANIFEST")
        .unwrap_or_else(|_| PACKAGING_PROVENANCE_MANIFEST_DEFAULT.into());
    let manifest_path = PathBuf::from(manifest);
    let manifest_text = read_required_text(&manifest_path, "provenance manifest")?;
    let pkg_dir = Path::new(PACKAGING_LAYOUT_DEFAULT);

    require_line_where(
        &manifest_text,
        |line| {
            matches!(
                line,
                "ghostty_source_mode=pinned-fetch" | "ghostty_source_mode=local"
            )
        },
        "Ghostty source mode",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| field_has_value(line, "GIT_CONFIG_GLOBAL="),
        "GIT_CONFIG_GLOBAL",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[toolchain]",
        "toolchain section",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("cargo=cargo "),
        "cargo version",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("rustc=rustc "),
        "rustc version",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "flatc=flatc version 25.12.19",
        "flatc version",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("zig=0.15."),
        "Zig 0.15 version",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[cargo_lock]",
        "Cargo.lock section",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        &format!(
            "Cargo.lock sha256={}",
            sha256_file(Path::new("Cargo.lock"))?
        ),
        "Cargo.lock hash",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[cargo_lock:libghostty-vt]",
        "locked libghostty-vt package section",
        &manifest_path,
    )?;
    require_lock_section(&manifest_text, "libghostty-vt", &manifest_path)?;
    require_exact(
        &manifest_text,
        "[cargo_lock:libghostty-vt-sys]",
        "locked libghostty-vt-sys package section",
        &manifest_path,
    )?;
    require_lock_section(&manifest_text, "libghostty-vt-sys", &manifest_path)?;

    require_exact(
        &manifest_text,
        "[package_metadata]",
        "package metadata section",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "package_format=local-tar-archive-layout",
        "package metadata format",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| field_has_value(line, "target_host="),
        "package metadata target host",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "terminal_engine=libghostty-vt",
        "package metadata terminal engine",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "terminal_engine_status=opt-in",
        "package metadata terminal engine status",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        RUNTIME_LIBRARY_STRATEGY,
        "package metadata runtime-library strategy",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| matches!(line, "source_mode=pinned-fetch" | "source_mode=local"),
        "package metadata source mode",
        &manifest_path,
    )?;

    require_exact(
        &manifest_text,
        "[staged_files]",
        "staged file section",
        &manifest_path,
    )?;
    require_file_record(&manifest_text, &pkg_dir.join("bin/nmux"), &manifest_path)?;
    require_file_record(
        &manifest_text,
        &pkg_dir.join("PACKAGE_METADATA.txt"),
        &manifest_path,
    )?;
    require_file_record(
        &manifest_text,
        &pkg_dir.join("libexec/nmux"),
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| {
            wildcard_file_record_is_valid(
                line,
                "target/packaging-libghostty-vt/package/lib/libghostty-vt",
            )
        },
        "libghostty-vt runtime library hash",
        &manifest_path,
    )?;

    require_exact(
        &manifest_text,
        "[native_runtime_libraries]",
        "native runtime library section",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("target/packaging-libghostty-vt/package/lib/libghostty-vt"),
        "native runtime library path",
        &manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[dynamic_dependencies]",
        "dynamic dependencies section",
        &manifest_path,
    )?;
    let nmux_bin = "target/packaging-libghostty-vt/package/libexec/nmux";
    require_exact(
        &manifest_text,
        nmux_bin,
        "nmux dynamic dependency heading",
        &manifest_path,
    )?;
    forbid_dynamic_dependency(&manifest_text, nmux_bin, "libghostty-vt", &manifest_path)?;
    require_exact(
        &manifest_text,
        "[cargo_tree]",
        "cargo tree section",
        &manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("nmux-cli v"),
        "nmux-cli cargo tree root",
        &manifest_path,
    )?;
    println!("provenance_manifest_verified={}", manifest_path.display());
    Ok(())
}

fn packaging_archive_verify() -> Result<()> {
    println!("verifying existing opt-in libghostty-vt package archive");
    let archive = PathBuf::from(
        env::var("PACKAGING_ARCHIVE").unwrap_or_else(|_| PACKAGING_ARCHIVE_DEFAULT.into()),
    );
    let archive_sha_file =
        PathBuf::from(env::var("PACKAGING_ARCHIVE_SHA256").unwrap_or_else(|_| {
            format!(
                "{}.sha256",
                archive.to_str().unwrap_or(PACKAGING_ARCHIVE_DEFAULT)
            )
        }));
    require_file(&archive, "archive artifact")?;
    require_file(&archive_sha_file, "archive SHA-256 artifact")?;

    let expected_sha = fs::read_to_string(&archive_sha_file)?
        .split_whitespace()
        .next()
        .ok_or_else(|| {
            format!(
                "invalid archive SHA-256 record: {}",
                archive_sha_file.display()
            )
        })?
        .to_owned();
    if !is_sha256(&expected_sha) {
        return Err(format!(
            "invalid archive SHA-256 record: {}",
            archive_sha_file.display()
        )
        .into());
    }
    let actual_sha = sha256_file(&archive)?;
    if actual_sha != expected_sha {
        return Err(format!(
            "archive SHA-256 mismatch: {}\nexpected {expected_sha}\nactual   {actual_sha}",
            archive.display()
        )
        .into());
    }
    verify_tar_paths_are_safe(&archive)?;

    let work_dir = TempDir::new("nmuxpkg-verify")?;
    extract_tar_gz(&archive, work_dir.path())?;
    let pkg_dir = work_dir.path().join("package");
    let metadata = pkg_dir.join("PACKAGE_METADATA.txt");
    let provenance = pkg_dir.join("PROVENANCE.txt");
    let cargo_tree = pkg_dir.join("CARGO_TREE.txt");
    require_file(&metadata, "package metadata")?;
    require_file(&provenance, "package provenance")?;
    require_file(&cargo_tree, "package cargo tree")?;

    let provenance_text = read_required_text(&provenance, "package provenance")?;
    require_archive_file_record(&pkg_dir, "bin/nmux", &provenance_text)?;
    require_archive_file_record(&pkg_dir, "PACKAGE_METADATA.txt", &provenance_text)?;
    require_archive_file_record(&pkg_dir, "CARGO_TREE.txt", &provenance_text)?;
    require_archive_file_record(&pkg_dir, "libexec/nmux", &provenance_text)?;
    let runtime_libraries = runtime_libraries(&pkg_dir.join("lib"))?;
    if runtime_libraries.is_empty() {
        return Err(format!(
            "missing libghostty-vt runtime library in archive: {}",
            archive.display()
        )
        .into());
    }
    for lib in runtime_libraries {
        let rel = format!("lib/{}", file_name(&lib)?);
        require_archive_file_record(&pkg_dir, &rel, &provenance_text)?;
    }

    let metadata_text = read_required_text(&metadata, "package metadata")?;
    require_exact(
        &metadata_text,
        "package_format=local-tar-archive-layout",
        "package metadata format",
        &metadata,
    )?;
    require_exact(
        &metadata_text,
        "terminal_engine=libghostty-vt",
        "package metadata terminal engine",
        &metadata,
    )?;
    require_exact(
        &metadata_text,
        "terminal_engine_status=opt-in",
        "package metadata terminal engine status",
        &metadata,
    )?;
    require_exact(
        &metadata_text,
        RUNTIME_LIBRARY_STRATEGY,
        "package metadata runtime-library strategy",
        &metadata,
    )?;
    for (line, description) in [
        ("[package_metadata]", "provenance package metadata section"),
        ("[staged_files]", "provenance staged file section"),
        (
            "[native_runtime_libraries]",
            "provenance runtime library section",
        ),
        (
            "[dynamic_dependencies]",
            "provenance dynamic dependencies section",
        ),
        ("[cargo_tree]", "provenance cargo tree section"),
        (
            "target/packaging-libghostty-vt/package/libexec/nmux",
            "nmux dynamic dependency heading",
        ),
    ] {
        require_exact(&provenance_text, line, description, &provenance)?;
    }
    forbid_dynamic_dependency(
        &provenance_text,
        "target/packaging-libghostty-vt/package/libexec/nmux",
        "libghostty-vt",
        &provenance,
    )?;
    let cargo_tree_text = read_required_text(&cargo_tree, "package cargo tree")?;
    require_line_where(
        &cargo_tree_text,
        |line| line.starts_with("nmux-cli v"),
        "cargo tree root",
        &cargo_tree,
    )?;
    packaging_layout_verify_for_dir(&pkg_dir)?;
    println!("packaging_archive_verified={}", archive.display());
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
            "missing record in {}: cargo lock section for {package}",
            report_path.display()
        )
        .into());
    }
    Ok(())
}

fn require_file_record(text: &str, path: &Path, report_path: &Path) -> Result<()> {
    let path = path.to_string_lossy();
    if text.lines().any(|line| {
        line.strip_prefix(path.as_ref())
            .is_some_and(file_record_suffix_is_valid)
    }) {
        return Ok(());
    }
    Err(format!(
        "missing staged file hash record in {}: {path}",
        report_path.display()
    )
    .into())
}

fn require_archive_file_record(pkg_dir: &Path, rel: &str, provenance_text: &str) -> Result<()> {
    let file = pkg_dir.join(rel);
    require_file(&file, "archive artifact")?;
    let bytes = fs::metadata(&file)?.len();
    let sha = sha256_file(&file)?;
    let staged = format!("target/packaging-libghostty-vt/package/{rel} bytes={bytes} sha256={sha}");
    if provenance_text.lines().any(|line| line == staged) {
        return Ok(());
    }
    Err(format!("missing or mismatched staged file hash record: target/packaging-libghostty-vt/package/{rel}").into())
}

fn file_record_suffix_is_valid(suffix: &str) -> bool {
    let Some(after_bytes) = suffix.strip_prefix(" bytes=") else {
        return false;
    };
    let Some((bytes, sha)) = after_bytes.split_once(" sha256=") else {
        return false;
    };
    !bytes.is_empty() && bytes.bytes().all(|byte| byte.is_ascii_digit()) && is_sha256(sha)
}

fn wildcard_file_record_is_valid(line: &str, path_prefix: &str) -> bool {
    let Some(suffix) = line.strip_prefix(path_prefix) else {
        return false;
    };
    let Some((filename_suffix, record_suffix)) = suffix.split_once(" bytes=") else {
        return false;
    };
    !filename_suffix.is_empty() && file_record_suffix_is_valid(&format!(" bytes={record_suffix}"))
}

fn forbid_dynamic_dependency(text: &str, bin: &str, dependency: &str, path: &Path) -> Result<()> {
    let mut in_bin = false;
    for line in report_section(text, "[dynamic_dependencies]").lines() {
        if line == bin || line == format!("{bin}:") {
            in_bin = true;
            continue;
        }
        if in_bin && line.starts_with("target/packaging-libghostty-vt/package/libexec/") {
            break;
        }
        if in_bin && line.contains(dependency) {
            return Err(format!(
                "unexpected dynamic dependency record in {} for {bin}: {dependency}",
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

fn verify_tar_paths_are_safe(archive: &Path) -> Result<()> {
    let output = Command::new("tar").arg("-tzf").arg(archive).output()?;
    if !output.status.success() {
        return Err(format!("tar list failed with status {}", output.status).into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    for entry in stdout.lines() {
        let path = Path::new(entry);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!(
                "archive contains unsafe absolute or parent-relative path: {entry}"
            )
            .into());
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-C")
        .arg(destination)
        .arg("-xzf")
        .arg(archive)
        .status()?;
    if !status.success() {
        return Err(format!("tar extract failed with status {status}").into());
    }
    Ok(())
}

fn runtime_libraries(lib_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut libraries = Vec::new();
    for entry in fs::read_dir(lib_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libghostty-vt"))
        {
            libraries.push(path);
        }
    }
    libraries.sort();
    Ok(libraries)
}

fn file_name(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("path has no UTF-8 file name: {}", path.display()).into())
}

fn packaging_layout_verify_for_dir(pkg_dir: &Path) -> Result<()> {
    let metadata = pkg_dir.join("PACKAGE_METADATA.txt");
    require_executable(&pkg_dir.join("bin/nmux"))?;
    require_executable(&pkg_dir.join("libexec/nmux"))?;
    require_file(&metadata, "package layout artifact")?;
    if runtime_libraries(&pkg_dir.join("lib"))?.is_empty() {
        return Err(format!(
            "missing libghostty-vt runtime library in package layout: {}",
            pkg_dir.join("lib").display()
        )
        .into());
    }
    let metadata_text = read_required_text(&metadata, "package metadata")?;
    require_exact(
        &metadata_text,
        "nmux opt-in native VT package metadata",
        "metadata title",
        &metadata,
    )?;
    require_line_where(
        &metadata_text,
        |line| {
            line.strip_prefix("generated_at_utc=")
                .is_some_and(is_utc_timestamp)
        },
        "metadata timestamp",
        &metadata,
    )?;
    for (line, description) in [
        ("package_format=local-tar-archive-layout", "package format"),
        (
            "release_status=local evidence artifact; not a signed, notarized, installed, or published release package",
            "release status",
        ),
        ("terminal_engine=libghostty-vt", "terminal engine"),
        ("terminal_engine_status=opt-in", "terminal engine status"),
        ("binaries=nmux", "binary list"),
        (RUNTIME_LIBRARY_STRATEGY, "runtime-library strategy"),
    ] {
        require_exact(&metadata_text, line, description, &metadata)?;
    }
    require_line_where(
        &metadata_text,
        |line| field_has_value(line, "target_host="),
        "target host",
        &metadata,
    )?;
    require_line_where(
        &metadata_text,
        |line| matches!(line, "source_mode=pinned-fetch" | "source_mode=local"),
        "source mode",
        &metadata,
    )?;
    require_line_where(
        &metadata_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR",
        &metadata,
    )?;
    require_wrapper_line(&pkg_dir.join("bin/nmux"), "#!/bin/sh", "shell shebang")?;
    require_wrapper_line(&pkg_dir.join("bin/nmux"), "set -eu", "strict shell mode")?;
    require_wrapper_line(
        &pkg_dir.join("bin/nmux"),
        r#"bin_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)"#,
        "relative wrapper directory",
    )?;
    require_wrapper_line(
        &pkg_dir.join("bin/nmux"),
        "lib_dir=$bin_dir/../lib",
        "relative library directory",
    )?;
    require_wrapper_line(
        &pkg_dir.join("bin/nmux"),
        "DYLD_LIBRARY_PATH=$lib_dir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}",
        "DYLD library path",
    )?;
    require_wrapper_line(
        &pkg_dir.join("bin/nmux"),
        "LD_LIBRARY_PATH=$lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}",
        "LD library path",
    )?;
    require_wrapper_line(
        &pkg_dir.join("bin/nmux"),
        "export DYLD_LIBRARY_PATH LD_LIBRARY_PATH",
        "library path export",
    )?;
    require_wrapper_line(
        &pkg_dir.join("bin/nmux"),
        r#"exec "$bin_dir/../libexec/nmux" "$@""#,
        "relative libexec handoff",
    )?;
    let status = Command::new(pkg_dir.join("bin/nmux"))
        .arg("--version")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .status()?;
    if !status.success() {
        return Err(format!("packaged nmux --version failed with status {status}").into());
    }
    Ok(())
}

fn require_executable(path: &Path) -> Result<()> {
    require_file(path, "package layout artifact")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "package layout artifact is not executable: {}",
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

fn require_wrapper_line(wrapper: &Path, line: &str, description: &str) -> Result<()> {
    let text = read_required_text(wrapper, "package wrapper")?;
    if text.lines().any(|candidate| candidate == line) {
        return Ok(());
    }
    Err(format!(
        "missing wrapper record in {}: {description}",
        wrapper.display()
    )
    .into())
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self> {
        let base = env::temp_dir();
        for attempt in 0..100 {
            let candidate = base.join(format!("{}-{}-{attempt}", prefix, std::process::id()));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self { path: candidate }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err.into()),
            }
        }
        Err(format!(
            "could not create temporary directory under {}",
            base.display()
        )
        .into())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
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
    Err(format!("missing record in {}: {description}", path.display()).into())
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
    Err(format!("missing record in {}: {description}", path.display()).into())
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
