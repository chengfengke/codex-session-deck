// Native executable shim: Windows cannot execute a Python shebang directly.
fn main() {
    let status = std::process::Command::new(std::env::var_os("DECK_TEST_PYTHON").unwrap())
        .arg(std::env::var_os("DECK_TEST_FIXTURE").unwrap())
        .args(std::env::args_os().skip(1))
        .status()
        .expect("launch fixture Python");
    std::process::exit(status.code().unwrap_or(1));
}
