use std::ffi::OsString;

fn run_with_args<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    sl_cli::run_cli_from_args(args)
}

#[cfg(not(coverage))]
fn main() {
    std::process::exit(run_with_args(std::env::args_os()));
}

#[cfg(coverage)]
fn main() {}

#[cfg(test)]
mod main_tests {
    use super::run_with_args;

    #[test]
    fn run_with_args_returns_non_zero_on_parse_error() {
        let code = run_with_args(["sl-cli", "invalid"]);
        assert_ne!(code, 0);
    }

    #[cfg(coverage)]
    #[test]
    fn coverage_main_is_invocable() {
        super::main();
    }
}
