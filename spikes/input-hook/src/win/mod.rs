mod args;
mod callback;
mod child;
mod identity;
mod input;
mod service;
mod supervisor;
mod token;
mod util;

pub fn entry() -> i32 {
    match args::parse() {
        Ok(args) => dispatch(args),
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}

fn dispatch(args: args::Args) -> i32 {
    if args.service {
        service::run_as_service()
    } else if let Some(role) = args.role.as_deref() {
        child::run(&args, role)
    } else {
        supervisor::run(&args)
    }
}
