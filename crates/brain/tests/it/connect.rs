//! The doors: `brain connect claude-code` registers the installed brain with Claude Code through
//! its own `claude mcp` command, and the Claude Desktop extension's copy hands over to the
//! installed brain. Each test runs with PATH holding only a stub `claude`, so the real one is
//! never found.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::Home;

/// An executable shell script at `path`.
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The install's `brain`, which here only writes its arguments to `argv` beside it.
fn install(h: &Home) -> PathBuf {
    let exe = h.home.path().join("data/bin/brain-current/brain");
    let argv = h.home.path().join("argv");
    script(&exe, &format!("echo \"$@\" > '{}'", argv.display()));
    exe
}

/// A folder holding a stub `claude` that appends each call's arguments to `calls` beside it.
fn stub_claude() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("calls");
    script(
        &dir.path().join("claude"),
        &format!("echo \"$@\" >> '{}'", calls.display()),
    );
    dir
}

fn connect(h: &Home, path: &Path, home: &Path) -> std::process::Output {
    h.brain()
        .env("PATH", path)
        .env("HOME", home)
        .args(["connect", "claude-code"])
        .output()
        .unwrap()
}

#[test]
fn connect_registers_the_installed_brain_for_claude_code() {
    let h = Home::new();
    let exe = install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = fs::read_to_string(stub.path().join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "mcp remove --scope user brain".to_string(),
            format!(
                "mcp add --scope user brain -- {} mcp --door claude-code",
                exe.display()
            )
        ]
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "Claude Code can now search your brain; start a new session."
    );
}

#[test]
fn connect_refuses_without_an_install() {
    let h = Home::new();
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("scripts/install.sh"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!stub.path().join("calls").exists(), "claude was never run");
}

#[test]
fn connect_writes_no_file() {
    let h = Home::new();
    install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    assert!(connect(&h, stub.path(), home.path()).status.success());
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn a_copy_outside_the_install_hands_over_to_the_installed_brain() {
    let h = Home::new();
    install(&h);
    let out = h
        .brain()
        .args(["mcp", "--door", "claude-desktop", "--hand-over"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(h.home.path().join("argv"))
            .unwrap()
            .trim(),
        "mcp --door claude-desktop"
    );
}

#[test]
fn hand_over_without_an_install_says_how_to_install() {
    let h = Home::new();
    let out = h
        .brain()
        .args(["mcp", "--door", "claude-desktop", "--hand-over"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "brain is not installed on this Mac; install it with scripts/install.sh, then run brain init"
    );
}

#[test]
fn the_mcpb_manifest_names_the_hand_over() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mcpb/manifest.json");
    let m: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(m["server"]["type"], "binary");
    assert_eq!(m["server"]["entry_point"], "server/brain");
    let args = m["server"]["mcp_config"]["args"].as_array().unwrap();
    assert_eq!(args.last().unwrap(), "--hand-over");
    assert_eq!(
        m["compatibility"]["platforms"],
        serde_json::json!(["darwin"])
    );
}
