mod ffi;
mod proof;

pub fn entry() -> i32 {
    match crate::args::parse_env() {
        Ok(args) => proof::dispatch(args),
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}
