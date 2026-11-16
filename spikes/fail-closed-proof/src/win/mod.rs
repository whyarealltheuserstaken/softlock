mod args;
mod notify;
mod policy;
mod proof;
mod service;
mod session;
mod util;

pub fn entry() -> i32 {
    match args::parse() {
        Ok(args) => proof::dispatch(args),
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}
