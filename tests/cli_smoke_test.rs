//! Black-box tests that run the real binary.
//!
//! These cover the argument parsing, exit codes and startup paths that live in `main.rs`, which
//! unit tests inside the library can never reach. `cargo llvm-cov` propagates its profiling
//! environment to child processes, so these count toward coverage of the binary target too.

use std::io::Write;
use std::process::{Command, Output};

/// Run the built binary with the given arguments and a clean environment.
///
/// The environment is cleared so a developer's exported `API_KEY_SALT` or a stray `.env`
/// cannot change the outcome.
fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_captchapi"));
    cmd.args(args).env_clear();
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("failed to run captchapi")
}

/// The two required parameters, so commands get far enough to be interesting.
fn secrets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("API_KEY_SALT", "smoke-test-salt-16chars"),
        ("MASTER_API_KEY", "smoke-test-master-key-16"),
    ]
}

fn temp_file(contents: &str, suffix: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(suffix)
        .tempfile()
        .expect("create temp file");
    f.write_all(contents.as_bytes()).expect("write temp file");
    f.flush().expect("flush temp file");
    f
}

#[test]
fn test_help_exits_zero_and_documents_the_flags() {
    let out = run(&["--help"], &[]);

    assert!(out.status.success(), "--help should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("USAGE:"), "{stdout}");
    assert!(stdout.contains("--port"), "{stdout}");
    assert!(stdout.contains("--captcha-compression"), "{stdout}");
    assert!(stdout.contains("config check"), "{stdout}");
}

#[test]
fn test_version_prints_the_crate_version() {
    let out = run(&["--version"], &[]);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "{stdout}");
}

#[test]
fn test_unknown_flag_exits_with_a_usage_error() {
    let out = run(&["--frobnicate"], &secrets());

    assert_eq!(out.status.code(), Some(2), "usage errors must exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--frobnicate"), "{stderr}");
    assert!(
        stderr.contains("--help"),
        "the error should point at --help: {stderr}"
    );
}

#[test]
fn test_unknown_command_exits_with_a_usage_error() {
    let out = run(&["frobnicate"], &secrets());

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown command"), "{stderr}");
}

#[test]
fn test_config_check_accepts_a_valid_configuration() {
    let toml = temp_file(
        "[server]\nport = 8080\n\n[captcha]\ncompression = 75\n",
        ".toml",
    );
    let out = run(
        &[
            "config",
            "check",
            "--no-env-file",
            "-c",
            toml.path().to_str().unwrap(),
        ],
        &secrets(),
    );

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("valid"));
}

#[test]
fn test_config_check_rejects_an_unknown_key_and_names_it() {
    let toml = temp_file("[server]\nprot = 8080\n", ".toml");
    let out = run(
        &[
            "config",
            "check",
            "--no-env-file",
            "-c",
            toml.path().to_str().unwrap(),
        ],
        &secrets(),
    );

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("server.prot"), "{stderr}");
    // The suggestion list is what turns a typo into a one-second fix.
    assert!(stderr.contains("server.port"), "{stderr}");
}

#[test]
fn test_config_check_reports_a_missing_required_secret() {
    let out = run(&["config", "check", "--no-env-file"], &[]);

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("API_KEY_SALT"), "{stderr}");
}

#[test]
fn test_config_check_reports_an_invalid_value() {
    let mut env = secrets();
    env.push(("SERVER_PORT", "not-a-port"));
    let out = run(&["config", "check", "--no-env-file"], &env);

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Invalid SERVER_PORT"), "{stderr}");
}

#[test]
fn test_config_show_reports_provenance_and_redacts_secrets() {
    let toml = temp_file("[captcha]\ncompression = 66\n", ".toml");
    let out = run(
        &[
            "config",
            "show",
            "--no-env-file",
            "-c",
            toml.path().to_str().unwrap(),
            "--port",
            "9191",
        ],
        &secrets(),
    );

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(stdout.contains("9191"), "{stdout}");
    assert!(
        stdout.contains("[cli]"),
        "the port came from a flag: {stdout}"
    );
    assert!(stdout.contains("66"), "{stdout}");
    assert!(
        stdout.contains("[file]"),
        "compression came from the file: {stdout}"
    );
    assert!(
        stdout.contains("[env]"),
        "the secrets came from the environment: {stdout}"
    );

    // The whole point of redaction.
    assert!(
        !stdout.contains("smoke-test-salt-16chars"),
        "config show leaked the salt: {stdout}"
    );
    assert!(
        !stdout.contains("smoke-test-master-key-16"),
        "config show leaked the master key: {stdout}"
    );
}

#[test]
fn test_command_line_beats_the_config_file() {
    let toml = temp_file("[server]\nport = 4444\n", ".toml");
    let out = run(
        &[
            "config",
            "show",
            "--no-env-file",
            "-c",
            toml.path().to_str().unwrap(),
            "--port",
            "5555",
        ],
        &secrets(),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("5555"), "{stdout}");
    assert!(
        !stdout.contains("4444"),
        "the file value should have lost: {stdout}"
    );
}

#[test]
fn test_secret_files_are_read_from_disk() {
    let salt = temp_file("salt-read-from-a-file-ok\n", ".secret");
    let key = temp_file("master-read-from-a-file\n", ".secret");

    let out = run(
        &[
            "config",
            "check",
            "--no-env-file",
            "--api-key-salt-file",
            salt.path().to_str().unwrap(),
            "--master-api-key-file",
            key.path().to_str().unwrap(),
        ],
        &[],
    );

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_missing_secret_file_is_reported() {
    let out = run(
        &[
            "config",
            "check",
            "--no-env-file",
            "--api-key-salt-file",
            "/nonexistent/salt",
        ],
        &secrets(),
    );

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot read secret file"), "{stderr}");
}

#[test]
fn test_reload_without_a_running_server_fails_cleanly() {
    let out = run(
        &["reload", "--pid-file", "/nonexistent/captchapi.pid"],
        &secrets(),
    );

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot read pid file"), "{stderr}");
    assert!(
        stderr.contains("--pid"),
        "should suggest the override: {stderr}"
    );
}

#[test]
fn test_env_file_is_read_as_a_layer() {
    let env_file = temp_file(
        "API_KEY_SALT=from-the-env-file-16ch\nMASTER_API_KEY=master-from-env-file-16\nSERVER_PORT=6161\n",
        ".env",
    );

    let out = run(
        &[
            "config",
            "show",
            "--env-file",
            env_file.path().to_str().unwrap(),
        ],
        &[],
    );

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("6161"), "{stdout}");
    assert!(stdout.contains("[env-file]"), "{stdout}");
}

#[test]
fn test_process_environment_beats_the_env_file() {
    // Preserves the precedence of the `dotenvy::dotenv()` call this replaced, which never
    // overwrote variables that were already set.
    let env_file = temp_file("SERVER_PORT=6161\n", ".env");
    let mut env = secrets();
    env.push(("SERVER_PORT", "7171"));

    let out = run(
        &[
            "config",
            "show",
            "--env-file",
            env_file.path().to_str().unwrap(),
        ],
        &env,
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("7171"), "{stdout}");
    assert!(
        !stdout.contains("6161"),
        "the env file should have lost: {stdout}"
    );
}
