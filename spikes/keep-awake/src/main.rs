fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    std::process::ExitCode::from(softlock_spike_keep_awake::run_from_args(&args, &cwd))
}
