use std::process;

fn main() {
    process::exit(sl_lint::run_from_args(std::env::args_os()));
}
