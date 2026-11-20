//! `harnessctl`: send commands to a running `photo --harness` session and
//! print the responses. See [`photo::harnessctl`].

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = photo::harnessctl::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(code)
}
