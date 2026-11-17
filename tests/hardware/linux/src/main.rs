use linux_hardware_suite::args::{Args, HELP};
use linux_hardware_suite::runner;

fn main() {
    let args = match Args::from_args() {
        Ok(a) if a.help => {
            print!("{HELP}");
            std::process::exit(0);
        }
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{HELP}");
            std::process::exit(2);
        }
    };
    let code = runner::run(args);
    std::process::exit(code);
}
