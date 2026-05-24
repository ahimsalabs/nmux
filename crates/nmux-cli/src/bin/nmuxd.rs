fn main() {
    if let Err(err) = nmux_cli::daemon::run_from_iter(std::env::args()) {
        eprintln!("nmuxd: {err}");
        std::process::exit(1);
    }
}
