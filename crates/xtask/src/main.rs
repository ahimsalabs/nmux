use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SOURCE_FETCH_REPORT_DEFAULT: &str = "target/source-fetch-provenance/SOURCE_FETCH.txt";
const SOURCE_FETCH_OFFLINE_PROBE_REPORT_DEFAULT: &str =
    "target/source-fetch-offline/OFFLINE_PROBE.txt";
const PACKAGING_PROVENANCE_MANIFEST_DEFAULT: &str =
    "target/packaging-libghostty-vt/package/PROVENANCE.txt";
const PACKAGING_LAYOUT_DEFAULT: &str = "target/packaging-libghostty-vt/package";
const PACKAGING_ARCHIVE_DEFAULT: &str =
    "target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz";
const PROMOTION_COLD_DEPS_DIR_DEFAULT: &str = "target/promotion-cold-deps";
const PROMOTION_EVIDENCE_DIR_DEFAULT: &str = "target/promotion-evidence";
const LIBGHOSTTY_VT_SOURCE: &str = r#"source = "git+https://github.com/uzaaft/libghostty-rs.git?rev=6a135cae68104ea8927f818cf8a1f5a13bcc8d9b#6a135cae68104ea8927f818cf8a1f5a13bcc8d9b""#;
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
        "packaging-layout-verify" => packaging_layout_verify(),
        "packaging-provenance-manifest-verify" => packaging_provenance_manifest_verify(),
        "promotion-cold-deps-verify" => promotion_cold_deps_verify(),
        "promotion-evidence-verify" => promotion_evidence_verify(),
        "static-link-verify" => static_link_verify(),
        "source-fetch-offline-probe-verify" => source_fetch_offline_probe_verify(),
        "source-fetch-provenance-verify" => source_fetch_provenance_verify(),
        "supply-chain-review" => supply_chain_review(),
        _ => usage_error(),
    }
}

fn usage_error() -> Result<()> {
    Err("usage: xtask <packaging-archive-verify|packaging-layout-verify|packaging-provenance-manifest-verify|promotion-cold-deps-verify|promotion-evidence-verify|static-link-verify|source-fetch-offline-probe-verify|source-fetch-provenance-verify|supply-chain-review>".into())
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
    let report =
        env::var("SOURCE_FETCH_REPORT").unwrap_or_else(|_| SOURCE_FETCH_REPORT_DEFAULT.into());
    source_fetch_provenance_verify_at(&PathBuf::from(report))
}

fn source_fetch_provenance_verify_at(report_path: &Path) -> Result<()> {
    println!("verifying source-fetch provenance report");
    let report_text = read_required_text(report_path, "source-fetch provenance artifact")?;

    require_exact(
        &report_text,
        "nmux source-fetch provenance sample",
        "report title",
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| {
            line.strip_prefix("generated_at_utc=")
                .is_some_and(is_utc_timestamp)
        },
        "generation timestamp",
        report_path,
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
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR field",
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| field_has_value(line, "GIT_CONFIG_GLOBAL="),
        "GIT_CONFIG_GLOBAL field",
        report_path,
    )?;
    require_exact(
        &report_text,
        &format!(
            "Cargo.lock sha256={}",
            sha256_file(Path::new("Cargo.lock"))?
        ),
        "Cargo.lock hash",
        report_path,
    )?;
    require_exact(
        &report_text,
        "[toolchain]",
        "toolchain section",
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| line.starts_with("cargo=cargo "),
        "cargo version",
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| line.starts_with("rustc=rustc "),
        "rustc version",
        report_path,
    )?;
    require_exact(
        &report_text,
        "flatc=flatc version 25.12.19",
        "flatc version",
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| line.starts_with("zig=0.15."),
        "Zig 0.15 version",
        report_path,
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
        report_path,
    )?;

    require_exact(
        &report_text,
        "[cargo_lock:libghostty-vt]",
        "libghostty-vt section",
        report_path,
    )?;
    require_exact(
        &report_text,
        r#"name = "libghostty-vt""#,
        "libghostty-vt package name",
        report_path,
    )?;
    require_exact(
        &report_text,
        LIBGHOSTTY_VT_SOURCE,
        "libghostty-vt pinned revision",
        report_path,
    )?;
    require_lock_section(&report_text, "libghostty-vt", report_path)?;

    require_exact(
        &report_text,
        "[cargo_lock:libghostty-vt-sys]",
        "libghostty-vt-sys section",
        report_path,
    )?;
    require_exact(
        &report_text,
        r#"name = "libghostty-vt-sys""#,
        "libghostty-vt-sys package name",
        report_path,
    )?;
    require_lock_section(&report_text, "libghostty-vt-sys", report_path)?;

    require_exact(
        &report_text,
        "[policy_note]",
        "policy note section",
        report_path,
    )?;
    require_exact(
        &report_text,
        "This report records local source-fetch inputs for evidence. It does not choose the default or packaged-build source policy.",
        "policy note",
        report_path,
    )?;
    println!("source_fetch_provenance_verified={}", report_path.display());
    Ok(())
}

fn source_fetch_offline_probe_verify() -> Result<()> {
    let report = env::var("SOURCE_FETCH_OFFLINE_PROBE_REPORT")
        .unwrap_or_else(|_| SOURCE_FETCH_OFFLINE_PROBE_REPORT_DEFAULT.into());
    let report_path = PathBuf::from(report);
    let report_text = read_required_text(&report_path, "source-fetch offline probe artifact")?;
    let log_path = match env::var("SOURCE_FETCH_OFFLINE_PROBE_LOG") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(
            field_value(&report_text, "log=")
                .ok_or_else(|| format!("missing record in {}: log path", report_path.display()))?,
        ),
    };
    source_fetch_offline_probe_verify_at(&report_path, &log_path)
}

fn source_fetch_offline_probe_verify_at(report_path: &Path, log_path: &Path) -> Result<()> {
    println!("verifying source-fetch offline probe report");
    let report_text = read_required_text(report_path, "source-fetch offline probe artifact")?;
    let log_text = read_required_text(log_path, "source-fetch offline probe log")?;

    require_exact(
        &report_text,
        "nmux source-fetch offline probe",
        "report title",
        report_path,
    )?;
    for (prefix, description) in [
        ("generated_at_utc=", "generation timestamp"),
        ("started_at_utc=", "start timestamp"),
        ("completed_at_utc=", "completion timestamp"),
    ] {
        require_line_where(
            &report_text,
            |line| line.strip_prefix(prefix).is_some_and(is_utc_timestamp),
            description,
            report_path,
        )?;
    }
    require_line_where(
        &report_text,
        |line| {
            line.strip_prefix("elapsed_seconds=")
                .is_some_and(is_decimal)
        },
        "elapsed seconds",
        report_path,
    )?;
    for (line, description) in [
        (
            "probe_scope=cache-present default native VT build only; not cold checkout, CI cache miss, or network-failure evidence",
            "probe scope",
        ),
        ("CARGO_NET_OFFLINE=true", "Cargo offline mode"),
        ("GIT_CONFIG_GLOBAL=/dev/null", "Git config isolation"),
        ("CARGO_TARGET_DIR=target/source-fetch-offline", "target dir"),
        ("package=nmux-core", "package"),
        ("features=default", "features"),
        ("command=cargo test -p nmux-core --no-run", "command"),
        (
            "log=target/source-fetch-offline/OFFLINE_PROBE.log",
            "log path",
        ),
        ("source_fetch_offline_probe=passed", "probe result"),
    ] {
        require_exact(&report_text, line, description, report_path)?;
    }
    require_line_where(
        &report_text,
        |line| {
            matches!(
                line,
                "ghostty_source_mode=pinned-fetch" | "ghostty_source_mode=local"
            )
        },
        "Ghostty source mode",
        report_path,
    )?;
    require_line_where(
        &report_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR field",
        report_path,
    )?;
    require_offline_probe_build_log(&log_text, log_path)?;
    println!(
        "source_fetch_offline_probe_verified={}",
        report_path.display()
    );
    Ok(())
}

fn packaging_provenance_manifest_verify() -> Result<()> {
    let manifest = env::var("PACKAGING_PROVENANCE_MANIFEST")
        .unwrap_or_else(|_| PACKAGING_PROVENANCE_MANIFEST_DEFAULT.into());
    packaging_provenance_manifest_verify_at(&PathBuf::from(manifest))
}

fn packaging_provenance_manifest_verify_at(manifest_path: &Path) -> Result<()> {
    println!("verifying default libghostty-vt package provenance manifest");
    let manifest_text = read_required_text(manifest_path, "provenance manifest")?;
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
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| field_has_value(line, "GIT_CONFIG_GLOBAL="),
        "GIT_CONFIG_GLOBAL",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[toolchain]",
        "toolchain section",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("cargo=cargo "),
        "cargo version",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("rustc=rustc "),
        "rustc version",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "flatc=flatc version 25.12.19",
        "flatc version",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("zig=0.15."),
        "Zig 0.15 version",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[cargo_lock]",
        "Cargo.lock section",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        &format!(
            "Cargo.lock sha256={}",
            sha256_file(Path::new("Cargo.lock"))?
        ),
        "Cargo.lock hash",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[cargo_lock:libghostty-vt]",
        "locked libghostty-vt package section",
        manifest_path,
    )?;
    require_lock_section(&manifest_text, "libghostty-vt", manifest_path)?;
    require_exact(
        &manifest_text,
        "[cargo_lock:libghostty-vt-sys]",
        "locked libghostty-vt-sys package section",
        manifest_path,
    )?;
    require_lock_section(&manifest_text, "libghostty-vt-sys", manifest_path)?;

    require_exact(
        &manifest_text,
        "[package_metadata]",
        "package metadata section",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "package_format=local-tar-archive-layout",
        "package metadata format",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| field_has_value(line, "target_host="),
        "package metadata target host",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "terminal_engine=libghostty-vt",
        "package metadata terminal engine",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "terminal_engine_status=opt-in",
        "package metadata terminal engine status",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        RUNTIME_LIBRARY_STRATEGY,
        "package metadata runtime-library strategy",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| matches!(line, "source_mode=pinned-fetch" | "source_mode=local"),
        "package metadata source mode",
        manifest_path,
    )?;

    require_exact(
        &manifest_text,
        "[staged_files]",
        "staged file section",
        manifest_path,
    )?;
    require_file_record(&manifest_text, &pkg_dir.join("bin/nmux"), manifest_path)?;
    require_file_record(
        &manifest_text,
        &pkg_dir.join("PACKAGE_METADATA.txt"),
        manifest_path,
    )?;
    require_file_record(&manifest_text, &pkg_dir.join("libexec/nmux"), manifest_path)?;
    require_line_where(
        &manifest_text,
        |line| {
            wildcard_file_record_is_valid(
                line,
                "target/packaging-libghostty-vt/package/lib/libghostty-vt",
            )
        },
        "libghostty-vt runtime library hash",
        manifest_path,
    )?;

    require_exact(
        &manifest_text,
        "[native_runtime_libraries]",
        "native runtime library section",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("target/packaging-libghostty-vt/package/lib/libghostty-vt"),
        "native runtime library path",
        manifest_path,
    )?;
    require_exact(
        &manifest_text,
        "[dynamic_dependencies]",
        "dynamic dependencies section",
        manifest_path,
    )?;
    let nmux_bin = "target/packaging-libghostty-vt/package/libexec/nmux";
    require_exact(
        &manifest_text,
        nmux_bin,
        "nmux dynamic dependency heading",
        manifest_path,
    )?;
    forbid_dynamic_dependency(&manifest_text, nmux_bin, "libghostty-vt", manifest_path)?;
    require_exact(
        &manifest_text,
        "[cargo_tree]",
        "cargo tree section",
        manifest_path,
    )?;
    require_line_where(
        &manifest_text,
        |line| line.starts_with("nmux-cli v"),
        "nmux-cli cargo tree root",
        manifest_path,
    )?;
    println!("provenance_manifest_verified={}", manifest_path.display());
    Ok(())
}

fn packaging_archive_verify() -> Result<()> {
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
    packaging_archive_verify_at(&archive, &archive_sha_file)
}

fn packaging_layout_verify() -> Result<()> {
    println!("verifying existing default libghostty-vt package layout");
    let pkg_dir = PathBuf::from(
        env::var("PACKAGING_LAYOUT").unwrap_or_else(|_| PACKAGING_LAYOUT_DEFAULT.into()),
    );
    packaging_layout_verify_for_dir(&pkg_dir)?;
    println!("packaging_layout_verified={}", pkg_dir.display());
    Ok(())
}

fn packaging_archive_verify_at(archive: &Path, archive_sha_file: &Path) -> Result<()> {
    println!("verifying existing default libghostty-vt package archive");
    require_file(archive, "archive artifact")?;
    require_file(archive_sha_file, "archive SHA-256 artifact")?;

    let expected_sha = fs::read_to_string(archive_sha_file)?
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
    let actual_sha = sha256_file(archive)?;
    if actual_sha != expected_sha {
        return Err(format!(
            "archive SHA-256 mismatch: {}\nexpected {expected_sha}\nactual   {actual_sha}",
            archive.display()
        )
        .into());
    }
    verify_tar_paths_are_safe(archive)?;

    let work_dir = TempDir::new("nmuxpkg-verify")?;
    extract_tar_gz(archive, work_dir.path())?;
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

fn promotion_cold_deps_verify() -> Result<()> {
    println!("verifying isolated Cargo dependency/source-fetch sample");
    let report_dir = Path::new(PROMOTION_COLD_DEPS_DIR_DEFAULT);
    let report = report_dir.join("REPORT.txt");
    let report_text = read_required_text(&report, "cold-deps artifact")?;
    let log_path = PathBuf::from(
        field_value(&report_text, "log=")
            .ok_or_else(|| format!("missing record in {}: log path", report.display()))?,
    );
    let log_text = read_required_text(&log_path, "cold-deps artifact")?;

    require_exact(
        &report_text,
        "nmux isolated Cargo dependency/source-fetch sample",
        "report title",
        &report,
    )?;
    for (prefix, description) in [
        ("started_at_utc=", "start timestamp"),
        ("completed_at_utc=", "completion timestamp"),
    ] {
        require_line_where(
            &report_text,
            |line| line.strip_prefix(prefix).is_some_and(is_utc_timestamp),
            description,
            &report,
        )?;
    }
    for (line, description) in [
        (
            "sample_scope=isolated repo-owned CARGO_HOME and CARGO_TARGET_DIR; Nix store, source checkout, and network state may still be warm",
            "sample scope",
        ),
        ("GHOSTTY_SOURCE_DIR=unset", "Ghostty source env"),
        ("GIT_CONFIG_GLOBAL=/dev/null", "Git config isolation"),
        ("command=just check-all", "sample command"),
        ("result=passed", "sample result"),
        ("log=target/promotion-cold-deps/RUN.log", "run log path"),
    ] {
        require_exact(&report_text, line, description, &report)?;
    }
    for (prefix, description) in [
        ("CARGO_HOME=", "isolated Cargo home"),
        ("CARGO_TARGET_DIR=", "isolated target dir"),
    ] {
        require_line_where(
            &report_text,
            |line| {
                line.strip_prefix(prefix).is_some_and(|value| {
                    value.ends_with(if prefix == "CARGO_HOME=" {
                        "/target/promotion-cold-deps/cargo-home"
                    } else {
                        "/target/promotion-cold-deps/target"
                    })
                })
            },
            description,
            &report,
        )?;
    }
    require_line_where(
        &report_text,
        |line| {
            line.strip_prefix("elapsed_seconds=")
                .is_some_and(is_decimal)
        },
        "elapsed seconds",
        &report,
    )?;

    let log_real = require_time_p_value(&log_text, "real", &log_path)?;
    let log_user = require_time_p_value(&log_text, "user", &log_path)?;
    let log_sys = require_time_p_value(&log_text, "sys", &log_path)?;
    for (line, description) in [
        (
            format!("check_all_real_seconds={log_real}"),
            "real timing matches log",
        ),
        (
            format!("check_all_user_seconds={log_user}"),
            "user timing matches log",
        ),
        (
            format!("check_all_sys_seconds={log_sys}"),
            "sys timing matches log",
        ),
    ] {
        require_exact(&report_text, &line, description, &report)?;
    }
    for (line, description) in [
        (
            "flatc --json --strict-json --no-warnings -o /tmp schema/nmux.fbs",
            "schema check ran",
        ),
        (
            "RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test --workspace",
            "default Ghostty workspace tests ran",
        ),
        (
            "cargo test -p nmux-core --no-default-features",
            "no-default-features core tests ran",
        ),
        (
            "cargo test -p nmux-cli --no-default-features",
            "no-default-features cli tests ran",
        ),
    ] {
        require_exact(&log_text, line, description, &log_path)?;
    }

    println!("promotion_cold_deps_verified={}", report.display());
    Ok(())
}

fn promotion_evidence_verify() -> Result<()> {
    println!("verifying local promotion evidence bundle");
    let bundle_dir = PathBuf::from(
        env::var("PROMOTION_EVIDENCE_DIR")
            .unwrap_or_else(|_| PROMOTION_EVIDENCE_DIR_DEFAULT.into()),
    );
    let summary = bundle_dir.join("SUMMARY.txt");
    let run_log = bundle_dir.join("RUN.log");
    let toolchain = bundle_dir.join("TOOLCHAIN.txt");
    let source_fetch = bundle_dir.join("SOURCE_FETCH.txt");
    let offline_probe = bundle_dir.join("OFFLINE_PROBE.txt");
    let package_provenance = bundle_dir.join("PACKAGE_PROVENANCE.txt");
    let promotion_open_work = bundle_dir.join("PROMOTION_OPEN_WORK.txt");
    let cargo_tree = bundle_dir.join("CARGO_TREE.txt");
    let archive_file = bundle_dir.join("PACKAGE_ARCHIVE.tar.gz");
    let archive_sha_file = bundle_dir.join("ARCHIVE.sha256");
    let cache_state = bundle_dir.join("CACHE_STATE.txt");
    let vcs_status = bundle_dir.join("VCS_STATUS.txt");
    let bundle_manifest = bundle_dir.join("BUNDLE_MANIFEST.txt");

    for (path, description) in [
        (&summary, "summary"),
        (&run_log, "run log"),
        (&toolchain, "toolchain"),
        (&source_fetch, "source-fetch"),
        (&offline_probe, "offline probe"),
        (&package_provenance, "package provenance"),
        (&promotion_open_work, "promotion open work"),
        (&cargo_tree, "cargo tree"),
        (&archive_file, "package archive"),
        (&archive_sha_file, "archive SHA-256"),
        (&cache_state, "cache state"),
        (&vcs_status, "VCS status"),
        (&bundle_manifest, "bundle manifest"),
    ] {
        require_file(path, description)?;
    }

    verify_promotion_bundle_manifest(&bundle_dir, &bundle_manifest)?;

    let summary_text = read_required_text(&summary, "summary")?;
    require_exact(
        &summary_text,
        "nmux promotion evidence bundle",
        "summary title",
        &summary,
    )?;
    for (prefix, description) in [
        ("generated_at_utc=", "generation timestamp"),
        ("started_at_utc=", "bundle start timestamp"),
        ("completed_at_utc=", "bundle completion timestamp"),
    ] {
        require_line_where(
            &summary_text,
            |line| line.strip_prefix(prefix).is_some_and(is_utc_timestamp),
            description,
            &summary,
        )?;
    }
    require_line_where(
        &summary_text,
        |line| {
            line.strip_prefix("bundle_elapsed_seconds=")
                .is_some_and(is_decimal)
        },
        "bundle elapsed seconds",
        &summary,
    )?;
    require_line_where(
        &summary_text,
        |line| field_has_value(line, "host="),
        "host identity",
        &summary,
    )?;
    require_line_where(
        &summary_text,
        |line| {
            line.strip_prefix("git_revision=")
                .is_some_and(is_unknown_or_git_sha)
        },
        "git revision",
        &summary,
    )?;
    require_line_where(
        &summary_text,
        |line| matches!(line, "github_actions=true" | "github_actions=false"),
        "GitHub Actions flag",
        &summary,
    )?;
    for (prefix, description) in [
        ("github_server_url=", "GitHub server URL field"),
        ("github_repository=", "GitHub repository field"),
        ("github_run_id=", "GitHub run ID field"),
        ("github_run_attempt=", "GitHub run attempt field"),
        ("github_ref=", "GitHub ref field"),
        ("github_sha=", "GitHub SHA field"),
        ("runner_os=", "runner OS field"),
        ("runner_arch=", "runner architecture field"),
        ("runner_name=", "runner name field"),
    ] {
        require_line_where(
            &summary_text,
            |line| field_has_value(line, prefix),
            description,
            &summary,
        )?;
    }
    let github_actions = field_value(&summary_text, "github_actions=").unwrap_or_default();
    if github_actions == "true" {
        require_line_where(
            &summary_text,
            |line| {
                line.strip_prefix("github_server_url=")
                    .is_some_and(|value| {
                        value.starts_with("http://") || value.starts_with("https://")
                    })
            },
            "GitHub Actions server URL",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| {
                line.strip_prefix("github_repository=")
                    .is_some_and(|value| value.split_once('/').is_some())
            },
            "GitHub Actions repository",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| line.strip_prefix("github_run_id=").is_some_and(is_decimal),
            "GitHub Actions run ID",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| {
                line.strip_prefix("github_run_attempt=")
                    .is_some_and(is_decimal)
            },
            "GitHub Actions run attempt",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| {
                line.strip_prefix("github_ref=")
                    .is_some_and(|value| value.starts_with("refs/"))
            },
            "GitHub Actions ref",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| line.strip_prefix("github_sha=").is_some_and(is_git_sha),
            "GitHub Actions SHA",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| {
                matches!(
                    line,
                    "runner_os=Linux" | "runner_os=macOS" | "runner_os=Windows"
                )
            },
            "GitHub Actions runner OS",
            &summary,
        )?;
        require_line_where(
            &summary_text,
            |line| {
                matches!(
                    line,
                    "runner_arch=X64" | "runner_arch=ARM64" | "runner_arch=X86"
                )
            },
            "GitHub Actions runner architecture",
            &summary,
        )?;
        require_absent_exact(
            &summary_text,
            "runner_name=unset",
            "GitHub Actions runner name must not be unset",
            &summary,
        )?;
    }
    require_line_where(
        &summary_text,
        |line| {
            matches!(
                line,
                "ghostty_source_mode=pinned-fetch" | "ghostty_source_mode=local"
            )
        },
        "Ghostty source mode",
        &summary,
    )?;
    require_line_where(
        &summary_text,
        |line| field_has_value(line, "GHOSTTY_SOURCE_DIR="),
        "GHOSTTY_SOURCE_DIR field",
        &summary,
    )?;
    require_line_where(
        &summary_text,
        |line| field_has_value(line, "GIT_CONFIG_GLOBAL="),
        "GIT_CONFIG_GLOBAL field",
        &summary,
    )?;
    require_line_where(
        &summary_text,
        |line| {
            matches!(
                line,
                "working_tree_status=clean"
                    | "working_tree_status=dirty"
                    | "working_tree_status=unknown"
            )
        },
        "working tree status",
        &summary,
    )?;
    for (line, description) in [
        ("vcs_status=VCS_STATUS.txt", "VCS status path"),
        (
            "promotion_open_work=PROMOTION_OPEN_WORK.txt",
            "promotion open work path",
        ),
        ("cache_state=CACHE_STATE.txt", "cache state path"),
        ("run_log=RUN.log", "run log path"),
        ("toolchain=TOOLCHAIN.txt", "toolchain path"),
        ("source_fetch=SOURCE_FETCH.txt", "source-fetch path"),
        (
            "source_fetch_offline_probe=OFFLINE_PROBE.txt",
            "source-fetch offline probe path",
        ),
        (
            "package_provenance=PACKAGE_PROVENANCE.txt",
            "package provenance path",
        ),
        ("cargo_tree=CARGO_TREE.txt", "cargo tree path"),
        (
            "package_archive=PACKAGE_ARCHIVE.tar.gz",
            "package archive path",
        ),
        (
            "local_smoke_reattach=passed",
            "local workflow persisted reattach smoke",
        ),
        (
            "local_smoke_socket_recreation=passed",
            "local workflow socket recreation smoke",
        ),
        (
            "local_smoke_print_context=passed",
            "local workflow print-context smoke",
        ),
        (
            "local_smoke_json_info=passed",
            "local workflow JSON informational smoke",
        ),
        (
            "local_smoke_ready_json=passed",
            "local workflow ready-json smoke",
        ),
        (
            "local_smoke_managed_start=passed",
            "local workflow managed start smoke",
        ),
        ("local_smoke=passed", "local workflow smoke"),
        ("packaged_runtime_smoke=passed", "packaged runtime smoke"),
    ] {
        require_exact(&summary_text, line, description, &summary)?;
    }
    for (prefix, description) in [
        ("check_all_real_seconds=", "check-all real timing"),
        ("check_all_user_seconds=", "check-all user timing"),
        ("check_all_sys_seconds=", "check-all sys timing"),
    ] {
        require_line_where(
            &summary_text,
            |line| line.strip_prefix(prefix).is_some_and(is_decimal),
            description,
            &summary,
        )?;
    }

    let run_log_text = read_required_text(&run_log, "run log")?;
    let check_all_real = require_time_p_value(&run_log_text, "real", &run_log)?;
    let check_all_user = require_time_p_value(&run_log_text, "user", &run_log)?;
    let check_all_sys = require_time_p_value(&run_log_text, "sys", &run_log)?;
    require_exact(
        &summary_text,
        &format!("check_all_real_seconds={check_all_real}"),
        "check-all real timing matches run log",
        &summary,
    )?;
    require_exact(
        &summary_text,
        &format!("check_all_user_seconds={check_all_user}"),
        "check-all user timing matches run log",
        &summary,
    )?;
    require_exact(
        &summary_text,
        &format!("check_all_sys_seconds={check_all_sys}"),
        "check-all sys timing matches run log",
        &summary,
    )?;
    let archive_sha = read_required_text(&archive_sha_file, "archive SHA-256")?;
    require_exact(
        &summary_text,
        &format!("archive_sha256={}", archive_sha.trim_end()),
        "archive SHA-256",
        &summary,
    )?;

    let toolchain_text = read_required_text(&toolchain, "toolchain")?;
    require_line_where(
        &toolchain_text,
        |line| line.starts_with("cargo=cargo "),
        "cargo version",
        &toolchain,
    )?;
    require_line_where(
        &toolchain_text,
        |line| line.starts_with("rustc=rustc "),
        "rustc version",
        &toolchain,
    )?;
    require_exact(
        &toolchain_text,
        "flatc=flatc version 25.12.19",
        "flatc version",
        &toolchain,
    )?;
    require_line_where(
        &toolchain_text,
        |line| line.starts_with("zig=0.15."),
        "Zig version",
        &toolchain,
    )?;

    source_fetch_provenance_verify_at(&source_fetch)?;
    verify_offline_probe(&offline_probe, &run_log, &run_log_text)?;
    verify_package_provenance_in_bundle(&package_provenance, &run_log_text, &run_log)?;
    packaging_provenance_manifest_verify_at(&package_provenance)?;
    require_line_where(
        &run_log_text,
        |line| {
            line == "provenance_manifest_verified=target/packaging-libghostty-vt/package/PROVENANCE.txt"
        },
        "package provenance verifier result",
        &run_log,
    )?;
    require_line_where(
        &run_log_text,
        |line| {
            line.starts_with("packaged_runtime_smoke_install_root=/tmp/nmuxpkg.")
                && line.ends_with("/install")
        },
        "relocated package install root",
        &run_log,
    )?;
    require_exact(
        &run_log_text,
        "packaged_runtime_smoke_library_env=unset",
        "clean packaged runtime library environment",
        &run_log,
    )?;
    require_exact(
        &run_log_text,
        "packaged_runtime_smoke=passed",
        "runtime smoke result",
        &run_log,
    )?;
    let cargo_tree_text = read_required_text(&cargo_tree, "cargo tree")?;
    require_line_where(
        &cargo_tree_text,
        |line| line.starts_with("nmux-cli v"),
        "cargo tree root",
        &cargo_tree,
    )?;
    require_line_where(
        &archive_sha,
        |line| {
            line.strip_suffix("  PACKAGE_ARCHIVE.tar.gz")
                .is_some_and(is_sha256)
        },
        "archive SHA-256 file",
        &archive_sha_file,
    )?;
    packaging_archive_verify_at(&archive_file, &archive_sha_file)?;
    verify_cache_state(&cache_state)?;
    verify_vcs_status(&vcs_status, &summary, &summary_text, &github_actions)?;
    verify_promotion_open_work(&promotion_open_work)?;
    println!("promotion_evidence_verified={}", summary.display());
    Ok(())
}

fn supply_chain_review() -> Result<()> {
    let base = supply_chain_base_ref()?;
    let head = env::var("SUPPLY_CHAIN_HEAD").unwrap_or_else(|_| "HEAD".to_owned());
    println!("supply_chain_review_base={base}");
    println!("supply_chain_review_head={head}");

    let changed_paths = git_changed_paths(&base, &head)?;
    let sensitive_paths = changed_paths
        .iter()
        .filter(|path| supply_chain_sensitive_path(path))
        .cloned()
        .collect::<Vec<_>>();

    println!(
        "supply_chain_sensitive_changes={}",
        if sensitive_paths.is_empty() {
            "false"
        } else {
            "true"
        }
    );

    if sensitive_paths.is_empty() {
        println!("supply_chain_review=not_required");
        return Ok(());
    }

    println!("supply_chain_review=required");
    println!(
        "supply_chain_review_required_reason=dependency pins, source-fetch policy, static-link policy, or packaging policy changed"
    );
    for path in &sensitive_paths {
        println!("supply_chain_path={path}");
    }

    if sensitive_paths.iter().any(|path| path == "Cargo.lock") {
        report_cargo_lock_changes(&base, &head)?;
    }

    println!(
        "supply_chain_reviewer_checklist=review changed pins, inspect upstream diffs for git rev updates, verify Cargo.lock and manifests agree, and confirm source/provenance/static-link CI remains green"
    );
    Ok(())
}

fn supply_chain_base_ref() -> Result<String> {
    if let Ok(base) = env::var("SUPPLY_CHAIN_BASE")
        && !base.is_empty()
    {
        return Ok(base);
    }
    if let Ok(base) = env::var("GITHUB_BASE_SHA")
        && !base.is_empty()
    {
        return Ok(base);
    }
    git_merge_base("origin/main", "HEAD").or_else(|_| Ok("HEAD^".to_owned()))
}

fn git_changed_paths(base: &str, head: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", base, head])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git diff --name-only {base} {head} failed with status {}",
            output.status
        )
        .into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn supply_chain_sensitive_path(path: &str) -> bool {
    path == "Cargo.lock"
        || path == "Cargo.toml"
        || path == "justfile"
        || path == ".github/workflows/check.yml"
        || path == ".github/workflows/nightly.yml"
        || path.starts_with("crates/") && path.ends_with("/Cargo.toml")
        || path == "docs/source-fetch-policy.md"
        || path == "docs/packaging.md"
        || path.starts_with("docs/adr/0024-")
        || path.starts_with("docs/adr/0025-")
        || path.starts_with("docs/adr/0026-")
        || path.starts_with("scripts/source-fetch-")
        || path.starts_with("scripts/packaging-")
        || path.starts_with("scripts/promotion-")
        || path == "crates/xtask/src/main.rs"
}

fn report_cargo_lock_changes(base: &str, head: &str) -> Result<()> {
    let old_lock = git_show_text(base, "Cargo.lock")?;
    let new_lock = git_show_text(head, "Cargo.lock")?;
    let old_packages = parse_cargo_lock_packages(&old_lock);
    let new_packages = parse_cargo_lock_packages(&new_lock);

    for new_package in &new_packages {
        let matches = old_packages
            .iter()
            .filter(|old_package| old_package.name == new_package.name)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => {
                println!(
                    "cargo_lock_added={} version={} source={}",
                    new_package.name,
                    new_package.version,
                    new_package.source.as_deref().unwrap_or("path")
                );
            }
            [old_package] => {
                if old_package.version != new_package.version
                    || old_package.source != new_package.source
                {
                    println!(
                        "cargo_lock_changed={} old_version={} new_version={} old_source={} new_source={}",
                        new_package.name,
                        old_package.version,
                        new_package.version,
                        old_package.source.as_deref().unwrap_or("path"),
                        new_package.source.as_deref().unwrap_or("path")
                    );
                    if let (Some(old_source), Some(new_source)) =
                        (&old_package.source, &new_package.source)
                        && let Some(compare_url) = git_source_compare_url(old_source, new_source)
                    {
                        println!(
                            "cargo_lock_upstream_compare={} {}",
                            new_package.name, compare_url
                        );
                    }
                }
            }
            _ => {
                println!("cargo_lock_changed_ambiguous={}", new_package.name);
            }
        }
    }

    for old_package in &old_packages {
        if !new_packages
            .iter()
            .any(|new_package| new_package.name == old_package.name)
        {
            println!(
                "cargo_lock_removed={} version={} source={}",
                old_package.name,
                old_package.version,
                old_package.source.as_deref().unwrap_or("path")
            );
        }
    }
    Ok(())
}

fn git_merge_base(left: &str, right: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["merge-base", left, right])
        .output()?;
    if !output.status.success() {
        return Err(format!("git merge-base {left} {right} failed").into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn git_show_text(revision: &str, path: &str) -> Result<String> {
    let spec = format!("{revision}:{path}");
    let output = Command::new("git").args(["show", &spec]).output()?;
    if !output.status.success() {
        return Err(format!("git show {spec} failed with status {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
}

fn parse_cargo_lock_packages(lock: &str) -> Vec<LockPackage> {
    lock.split("[[package]]")
        .skip(1)
        .filter_map(parse_cargo_lock_package)
        .collect()
}

fn parse_cargo_lock_package(block: &str) -> Option<LockPackage> {
    let name = cargo_lock_string_field(block, "name")?;
    let version = cargo_lock_string_field(block, "version")?;
    let source = cargo_lock_string_field(block, "source");
    Some(LockPackage {
        name,
        version,
        source,
    })
}

fn cargo_lock_string_field(block: &str, field: &str) -> Option<String> {
    let prefix = format!("{field} = ");
    block
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.strip_prefix('"')?.strip_suffix('"'))
        .map(str::to_owned)
}

fn git_source_compare_url(old_source: &str, new_source: &str) -> Option<String> {
    let old = parse_git_source(old_source)?;
    let new = parse_git_source(new_source)?;
    if old.repo != new.repo || old.rev == new.rev {
        return None;
    }
    github_compare_url(&new.repo, &old.rev, &new.rev)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSource {
    repo: String,
    rev: String,
}

fn parse_git_source(source: &str) -> Option<GitSource> {
    let source = source.strip_prefix("git+")?;
    let (repo, fragment) = source.rsplit_once('#')?;
    let repo = repo.split_once('?').map_or(repo, |(repo, _)| repo);
    Some(GitSource {
        repo: repo.to_owned(),
        rev: fragment.to_owned(),
    })
}

fn github_compare_url(repo: &str, old_rev: &str, new_rev: &str) -> Option<String> {
    let path = repo.strip_prefix("https://github.com/")?;
    let path = path.strip_suffix(".git").unwrap_or(path);
    Some(format!(
        "https://github.com/{path}/compare/{old_rev}...{new_rev}"
    ))
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
        "nmux default native VT package metadata",
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
        ("terminal_engine_status=default", "terminal engine status"),
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
    let output = Command::new(pkg_dir.join("bin/nmux"))
        .arg("--version")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "packaged nmux --version failed with status {}",
            output.status
        )
        .into());
    }
    let version = String::from_utf8(output.stdout)?;
    println!(
        "packaged libghostty-vt nmux version: {}",
        version.trim_end()
    );
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

fn verify_promotion_bundle_manifest(bundle_dir: &Path, manifest: &Path) -> Result<()> {
    const NAMES: &[&str] = &[
        "ARCHIVE.sha256",
        "CACHE_STATE.txt",
        "CARGO_TREE.txt",
        "OFFLINE_PROBE.txt",
        "PACKAGE_ARCHIVE.tar.gz",
        "PACKAGE_PROVENANCE.txt",
        "PROMOTION_OPEN_WORK.txt",
        "RUN.log",
        "SOURCE_FETCH.txt",
        "SUMMARY.txt",
        "TOOLCHAIN.txt",
        "VCS_STATUS.txt",
    ];
    let mut expected = String::from("nmux promotion evidence bundle manifest\n");
    for name in NAMES {
        expected.push_str(&format!(
            "{}  {name}\n",
            sha256_file(&bundle_dir.join(name))?
        ));
    }
    let actual = fs::read_to_string(manifest)?;
    if actual != expected {
        return Err(format!("bundle manifest mismatch: {}", manifest.display()).into());
    }
    Ok(())
}

fn require_time_p_value(run_log: &str, field: &str, path: &Path) -> Result<String> {
    let mut value = None;
    for line in run_log.lines() {
        if line == "== promotion local sample: packaging archive runtime smoke ==" {
            break;
        }
        if let Some(candidate) = line.strip_prefix(&format!("{field} "))
            && is_decimal(candidate)
        {
            value = Some(candidate.to_owned());
        }
    }
    value.ok_or_else(|| {
        format!(
            "missing check-all {field} time -p result in {}",
            path.display()
        )
        .into()
    })
}

fn verify_offline_probe(offline_probe: &Path, run_log: &Path, run_log_text: &str) -> Result<()> {
    let text = read_required_text(offline_probe, "offline probe")?;
    for (line, description) in [
        ("nmux source-fetch offline probe", "offline probe title"),
        (
            "probe_scope=cache-present default native VT build only; not cold checkout, CI cache miss, or network-failure evidence",
            "offline probe scope",
        ),
        ("CARGO_NET_OFFLINE=true", "offline probe Cargo offline mode"),
        (
            "GIT_CONFIG_GLOBAL=/dev/null",
            "offline probe Git config isolation",
        ),
        (
            "CARGO_TARGET_DIR=target/source-fetch-offline",
            "offline probe target dir",
        ),
        ("package=nmux-core", "offline probe package"),
        ("features=default", "offline probe features"),
        ("source_fetch_offline_probe=passed", "offline probe result"),
    ] {
        require_exact(&text, line, description, offline_probe)?;
    }
    require_line_where(
        &text,
        |line| {
            line.strip_prefix("elapsed_seconds=")
                .is_some_and(is_decimal)
        },
        "offline probe elapsed seconds",
        offline_probe,
    )?;
    for (line, description) in [
        (
            "== promotion local sample: cache-present offline source-fetch probe ==",
            "offline probe run-log section",
        ),
        (
            "verifying source-fetch offline probe report",
            "offline probe verifier ran",
        ),
        (
            "source_fetch_offline_probe_verified=target/source-fetch-offline/OFFLINE_PROBE.txt",
            "offline probe verifier result",
        ),
        (
            "source_fetch_offline_probe=target/source-fetch-offline/OFFLINE_PROBE.txt",
            "offline probe artifact path",
        ),
    ] {
        require_exact(run_log_text, line, description, run_log)?;
    }
    require_line_where(
        run_log_text,
        |line| line == "running cache-present source-fetch offline probe",
        "offline probe command ran",
        run_log,
    )?;
    source_fetch_offline_probe_verify_at(offline_probe, run_log)
}

fn require_offline_probe_build_log(log_text: &str, log_path: &Path) -> Result<()> {
    require_line_where(
        log_text,
        |line| {
            line == "   Compiling libghostty-vt-sys v0.1.1"
                || line == "    Checking libghostty-vt-sys v0.1.1"
                || line.starts_with("    Finished `test` profile ")
        },
        "offline probe native VT build activity",
        log_path,
    )?;
    require_line_where(
        log_text,
        |line| {
            line.starts_with("  Executable unittests src/lib.rs (target/source-fetch-offline/debug/deps/nmux_core-")
                && line.ends_with(')')
        },
        "offline probe nmux-core test binary",
        log_path,
    )
}

fn verify_package_provenance_in_bundle(
    package_provenance: &Path,
    run_log_text: &str,
    run_log: &Path,
) -> Result<()> {
    let text = read_required_text(package_provenance, "package provenance")?;
    for (line, description) in [
        ("[staged_files]", "packaging staged file hashes"),
        ("[native_runtime_libraries]", "packaging runtime libraries"),
        (
            "package_format=local-tar-archive-layout",
            "package metadata format",
        ),
        (
            "terminal_engine=libghostty-vt",
            "package metadata terminal engine",
        ),
        (
            "terminal_engine_status=opt-in",
            "package metadata terminal engine status",
        ),
        ("[dynamic_dependencies]", "packaging dynamic dependencies"),
    ] {
        require_exact(&text, line, description, package_provenance)?;
    }
    for (prefix, description) in [
        (
            "target/packaging-libghostty-vt/package/bin/nmux",
            "packaged nmux wrapper hash",
        ),
        (
            "target/packaging-libghostty-vt/package/PACKAGE_METADATA.txt",
            "package metadata hash",
        ),
        (
            "target/packaging-libghostty-vt/package/libexec/nmux",
            "packaged nmux binary hash",
        ),
    ] {
        require_line_where(
            &text,
            |line| {
                line.strip_prefix(prefix)
                    .is_some_and(file_record_suffix_is_valid)
            },
            description,
            package_provenance,
        )?;
    }
    require_line_where(
        &text,
        |line| {
            wildcard_file_record_is_valid(
                line,
                "target/packaging-libghostty-vt/package/lib/libghostty-vt",
            )
        },
        "packaged native runtime library hash",
        package_provenance,
    )?;
    require_line_where(
        &text,
        |line| line.starts_with("target/packaging-libghostty-vt/package/lib/libghostty-vt"),
        "packaging runtime library path",
        package_provenance,
    )?;
    for (line, description) in [
        (
            "== promotion local sample: local workflow smoke ==",
            "local smoke run-log section",
        ),
        ("running local nmux daemon/client smoke", "local smoke ran"),
        (
            "local_smoke_reattach=passed",
            "local smoke persisted reattach result",
        ),
        (
            "local_smoke_socket_recreation=passed",
            "local smoke socket recreation result",
        ),
        (
            "local_smoke_print_context=passed",
            "local smoke print-context result",
        ),
        (
            "local_smoke_json_info=passed",
            "local smoke JSON informational result",
        ),
        (
            "local_smoke_ready_json=passed",
            "local smoke ready-json result",
        ),
        (
            "local_smoke_managed_start=passed",
            "local smoke managed start result",
        ),
        ("local_smoke=passed", "local smoke result"),
    ] {
        require_exact(run_log_text, line, description, run_log)?;
    }
    Ok(())
}

fn verify_cache_state(cache_state: &Path) -> Result<()> {
    let text = read_required_text(cache_state, "cache state")?;
    require_exact(
        &text,
        "nmux promotion evidence cache state",
        "cache state title",
        cache_state,
    )?;
    require_line_where(
        &text,
        |line| {
            line.strip_prefix("generated_at_utc=")
                .is_some_and(is_utc_timestamp)
        },
        "cache state timestamp",
        cache_state,
    )?;
    for (prefix, description) in [
        ("cache_state_scope=", "cache state scope"),
        ("CARGO_HOME=", "cache state CARGO_HOME"),
        ("CARGO_TARGET_DIR=", "cache state CARGO_TARGET_DIR"),
        ("cache_state_note=", "cache state interpretation note"),
    ] {
        require_line_where(
            &text,
            |line| field_has_value(line, prefix),
            description,
            cache_state,
        )?;
    }
    for (prefix, description) in [
        ("nix_store_status=", "Nix store cache status"),
        ("cargo_home_status=", "Cargo home cache status"),
        ("cargo_registry_status=", "Cargo registry cache status"),
        ("cargo_git_status=", "Cargo git cache status"),
        ("cargo_target_dir_status=", "Cargo target cache status"),
        (
            "packaging_libghostty_vt_target_dir_status=",
            "native VT target cache status",
        ),
        (
            "source_fetch_offline_probe_status=",
            "source-fetch offline probe status",
        ),
    ] {
        require_line_where(
            &text,
            |line| {
                line.strip_prefix(prefix)
                    .is_some_and(|value| matches!(value, "present" | "missing"))
            },
            description,
            cache_state,
        )?;
    }
    Ok(())
}

fn verify_vcs_status(
    vcs_status: &Path,
    summary: &Path,
    summary_text: &str,
    github_actions: &str,
) -> Result<()> {
    let text = read_required_text(vcs_status, "VCS status")?;
    require_exact(
        &text,
        "nmux promotion evidence VCS status",
        "VCS status title",
        vcs_status,
    )?;
    require_line_where(
        &text,
        |line| {
            line.strip_prefix("generated_at_utc=")
                .is_some_and(is_utc_timestamp)
        },
        "VCS status timestamp",
        vcs_status,
    )?;
    require_line_where(
        &text,
        |line| field_has_value(line, "vcs_status_scope="),
        "VCS status scope",
        vcs_status,
    )?;
    require_line_where(
        &text,
        |line| {
            line.strip_prefix("git_revision=")
                .is_some_and(is_unknown_or_git_sha)
        },
        "VCS git revision",
        vcs_status,
    )?;
    require_line_where(
        &text,
        |line| {
            line.strip_prefix("git_status_porcelain=")
                .is_some_and(|value| matches!(value, "clean" | "dirty" | "unknown"))
        },
        "VCS git working-tree status",
        vcs_status,
    )?;
    require_exact(
        &text,
        "[git_status_porcelain_v1]",
        "VCS git status section",
        vcs_status,
    )?;
    require_line_where(
        &text,
        |line| {
            matches!(
                line,
                "jj_status_available=true" | "jj_status_available=false"
            )
        },
        "VCS jj availability",
        vcs_status,
    )?;
    require_exact(&text, "[jj_status]", "VCS jj status section", vcs_status)?;
    let summary_git_revision = field_value(summary_text, "git_revision=").unwrap_or_default();
    let vcs_git_revision = field_value(&text, "git_revision=").unwrap_or_default();
    if summary_git_revision != vcs_git_revision {
        return Err(format!(
            "promotion evidence git revision mismatch: SUMMARY.txt has {summary_git_revision} but VCS_STATUS.txt has {vcs_git_revision}"
        )
        .into());
    }
    if github_actions == "true" {
        let github_sha = field_value(summary_text, "github_sha=").unwrap_or_default();
        if summary_git_revision != github_sha {
            return Err(format!(
                "promotion evidence GitHub SHA mismatch: SUMMARY.txt git_revision is {summary_git_revision} but github_sha is {github_sha}"
            )
            .into());
        }
    }
    let vcs_worktree_status = field_value(&text, "git_status_porcelain=").unwrap_or_default();
    require_exact(
        summary_text,
        &format!("working_tree_status={vcs_worktree_status}"),
        "summary working tree status matches VCS artifact",
        summary,
    )
}

fn verify_promotion_open_work(promotion_open_work: &Path) -> Result<()> {
    let text = read_required_text(promotion_open_work, "promotion open work")?;
    for (line, description) in [
        (
            "nmux native VT promotion open work",
            "promotion open work title",
        ),
        ("promotion_decision=promoted", "promotion decision"),
        (
            "default_terminal_engine=libghostty-vt",
            "default terminal engine",
        ),
        ("libghostty_vt_status=default", "libghostty-vt status"),
        (
            "open_work_scope=post-promotion release evidence and distribution work that remains after libghostty-vt became the default engine",
            "promotion open work scope",
        ),
        (
            "open_work_ci=manual promotion evidence bundle and downloaded-artifact verifier jobs remain release evidence rather than default-engine blockers",
            "promotion open work CI gap",
        ),
        (
            "open_work_platforms=more supported local systems and at least one full cold-checkout or cold-machine run still need timing evidence",
            "promotion open work platform gap",
        ),
        (
            "open_work_non_nix=non-Nix toolchain checklist still needs a successful platform-specific validation run",
            "promotion open work non-Nix gap",
        ),
        (
            "open_work_source_policy=flake packages use a pinned Nix-fetched Ghostty source; non-Nix and release artifact source policies still need platform-specific evidence",
            "promotion open work source-policy gap",
        ),
        (
            "open_work_packaging=native VT binary distribution expectations still need supported-target, signing, notarization, installed-package, and platform distribution decisions despite local layout, archive, provenance, and runtime-smoke verifiers",
            "promotion open work packaging gap",
        ),
        (
            "open_work_frontend=frontend Ghostty renderer hydration remains separate from backend terminal-state extraction",
            "promotion open work frontend gap",
        ),
    ] {
        require_exact(&text, line, description, promotion_open_work)?;
    }
    require_line_where(
        &text,
        |line| {
            line.strip_prefix("generated_at_utc=")
                .is_some_and(is_utc_timestamp)
        },
        "promotion open work timestamp",
        promotion_open_work,
    )
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

fn require_absent_exact(text: &str, line: &str, description: &str, path: &Path) -> Result<()> {
    if text.lines().any(|candidate| candidate == line) {
        return Err(format!("unexpected record in {}: {description}", path.display()).into());
    }
    Ok(())
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

fn field_value(text: &str, prefix: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_owned))
}

fn is_decimal(value: &str) -> bool {
    let Some((whole, fraction)) = value.split_once('.') else {
        return !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    };
    !whole.is_empty()
        && !fraction.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_git_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_unknown_or_git_sha(value: &str) -> bool {
    value == "unknown" || is_git_sha(value)
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
