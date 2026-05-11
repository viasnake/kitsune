use std::env;
use std::process;

fn main() {
    if let Err(error) = kitsunebi_core::cli::run(env::args().skip(1).collect()) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}
