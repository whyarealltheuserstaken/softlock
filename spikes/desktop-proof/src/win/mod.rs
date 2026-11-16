mod args;
mod capture;
mod diag;
mod hook;
mod proof;
mod service;
mod util;
mod window;

pub fn entry() -> i32 {
    match args::parse() {
        Ok(args) => proof::dispatch(args),
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}
