fn main() {
    let config = match softlock_spike_overlay::Config::parse(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = softlock_spike_overlay::run(config) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
