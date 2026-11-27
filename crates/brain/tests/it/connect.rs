//! The doors: `brain connect claude-code` registers the installed brain with Claude Code through
//! its own `claude mcp` command, and the Claude Desktop extension's copy hands over to the
//! installed brain. Each test runs with PATH holding only a stub `claude`, so the real one is
//! never found.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::Home;

/// What every "not installed" message tells a person to do: the package on macOS, the script
/// elsewhere.
const HOW_TO_INSTALL: &str = if cfg!(target_os = "macos") {
    "install brain from brain.pkg"
} else {
    "install brain with scripts/install.sh"
};

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
            ),
            "plugin marketplace remove brain".to_string(),
            format!("plugin marketplace add {}", plugin(&h).display()),
            "plugin install brain@brain".to_string(),
        ]
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "Claude Code can now search your brain; start a new session."
    );
}

fn plugin(h: &Home) -> PathBuf {
    h.home.path().join("data/claude-plugin")
}

fn json(path: PathBuf) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
}

fn command(h: &Home, event: &str) -> String {
    json(plugin(h).join("brain/hooks/hooks.json"))["hooks"][event][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn connect_installs_the_plugin_beside_the_mcp_server() {
    let h = Home::new();
    let exe = install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    // A second connect rewrites the same folder.
    assert!(connect(&h, stub.path(), home.path()).status.success());
    assert!(connect(&h, stub.path(), home.path()).status.success());
    let version = env!("CARGO_PKG_VERSION");
    let market = json(plugin(&h).join(".claude-plugin/marketplace.json"));
    assert_eq!(market["name"], "brain");
    assert_eq!(market["plugins"][0]["name"], "brain");
    assert_eq!(market["plugins"][0]["source"], "./brain");
    let manifest = json(plugin(&h).join("brain/.claude-plugin/plugin.json"));
    assert_eq!(manifest["name"], "brain");
    assert_eq!(manifest["version"], version);
    for (event, point) in [
        ("SessionStart", "session-start"),
        ("UserPromptSubmit", "prompt"),
        ("Stop", "stop"),
        ("SessionEnd", "session-end"),
    ] {
        let expected = format!("'{}' hook {point} || true", exe.display());
        assert_eq!(command(&h, event), expected);
    }
    let skill = fs::read_to_string(plugin(&h).join("brain/skills/brain/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: brain\n"), "{skill}");
}

#[test]
fn a_failed_plugin_install_says_the_hooks_are_not_connected() {
    let h = Home::new();
    install(&h);
    let stub = tempfile::tempdir().unwrap();
    script(
        &stub.path().join("claude"),
        "case \"$1 $2\" in 'plugin install') exit 3;; esac",
    );
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("can search your brain, but the hooks are not connected"),
        "{said}"
    );
}

/// The macOS data folder is under `Application Support`.
#[test]
fn the_hook_command_runs_from_a_path_with_a_space() {
    let h = Home {
        home: tempfile::Builder::new()
            .prefix("brain home ")
            .tempdir()
            .unwrap(),
        vault: tempfile::tempdir().unwrap(),
    };
    let exe = install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    assert!(connect(&h, stub.path(), home.path()).status.success());
    let ran = std::process::Command::new("/bin/sh")
        .args(["-c", &command(&h, "UserPromptSubmit")])
        .status()
        .unwrap();
    assert!(ran.success());
    let argv = fs::read_to_string(h.home.path().join("argv")).unwrap();
    assert_eq!(argv.trim(), "hook prompt");
    assert!(exe.exists());
}

#[test]
fn connect_refuses_a_path_the_shell_would_break_on() {
    let h = Home {
        home: tempfile::Builder::new()
            .prefix("o'brien ")
            .tempdir()
            .unwrap(),
        vault: tempfile::tempdir().unwrap(),
    };
    install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("single quote"));
    assert!(!plugin(&h).exists());
}

#[test]
fn connect_refuses_without_an_install() {
    let h = Home::new();
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(HOW_TO_INSTALL),
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
        format!("brain is not installed on this computer; {HOW_TO_INSTALL}, then run brain init")
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

/// A stub `claude` that writes each argument on its own line to `calls`, and exits 1 when its
/// second argument is `fail` (`add` or `remove`).
fn claude_failing_on(fail: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("calls");
    script(
        &dir.path().join("claude"),
        &format!(
            "for a in \"$@\"; do echo \"$a\"; done >> '{}'\n[ \"$2\" = '{fail}' ] && exit 1\nexit 0",
            calls.display()
        ),
    );
    dir
}

#[test]
fn connect_fails_when_claude_add_fails() {
    let h = Home::new();
    install(&h);
    let stub = claude_failing_on("add");
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("can now search"));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("run brain connect claude-code again"), "{err}");
}

#[test]
fn connect_carries_on_when_nothing_was_connected_before() {
    let h = Home::new();
    install(&h);
    let stub = claude_failing_on("remove");
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn connect_names_claude_when_it_is_missing() {
    let h = Home::new();
    install(&h);
    let empty = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, empty.path(), home.path());
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("is Claude Code installed"), "{err}");
}

#[test]
fn an_install_path_with_spaces_is_one_argument() {
    let h = Home::new();
    let spaced = h.home.path().join("my brain home");
    let exe = spaced.join("data/bin/brain-current/brain");
    script(&exe, "exit 0");
    let stub = claude_failing_on("none");
    let home = tempfile::tempdir().unwrap();
    let out = h
        .brain()
        .env("BRAIN_HOME", &spaced)
        .env("PATH", stub.path())
        .env("HOME", home.path())
        .args(["connect", "claude-code"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = fs::read_to_string(stub.path().join("calls")).unwrap();
    assert!(calls.lines().any(|l| l == exe.to_str().unwrap()), "{calls}");
}

#[test]
fn a_hand_over_to_an_install_that_cannot_run_fails() {
    let h = Home::new();
    let exe = h.home.path().join("data/bin/brain-current/brain");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(&exe, "not a program").unwrap();
    let out = h
        .brain()
        .args(["mcp", "--door", "claude-desktop", "--hand-over"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("starting"), "{err}");
}

#[test]
fn connect_as_registers_the_name() {
    let h = Home::new();
    let exe = install(&h);
    let stub = claude_failing_on("none");
    let home = tempfile::tempdir().unwrap();
    let out = h
        .brain()
        .env("PATH", stub.path())
        .env("HOME", home.path())
        .args(["connect", "claude-code", "--as", "chief of staff"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = fs::read_to_string(stub.path().join("calls")).unwrap();
    let add: Vec<&str> = calls
        .lines()
        .skip_while(|l| *l != "add")
        .take_while(|l| *l != "plugin")
        .collect();
    assert_eq!(
        add,
        [
            "add",
            "--scope",
            "user",
            "brain",
            "--",
            exe.to_str().unwrap(),
            "mcp",
            "--door",
            "claude-code",
            "--as=chief of staff",
        ]
    );
}

#[test]
fn connect_refuses_a_bad_name_before_running_claude() {
    let h = Home::new();
    install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    let out = h
        .brain()
        .env("PATH", stub.path())
        .env("HOME", home.path())
        .args(["connect", "claude-code", "--as", "two\nlines"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("a name is"));
    assert!(!stub.path().join("calls").exists(), "claude was never run");
}

#[test]
fn a_hand_over_keeps_the_name() {
    let h = Home::new();
    install(&h);
    let out = h
        .brain()
        .args([
            "mcp",
            "--door",
            "claude-desktop",
            "--hand-over",
            "--as",
            "chief",
        ])
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
        "mcp --door claude-desktop --as=chief"
    );
}

#[test]
fn connect_as_a_name_starting_with_a_dash_registers_one_word() {
    let h = Home::new();
    install(&h);
    let stub = claude_failing_on("none");
    let home = tempfile::tempdir().unwrap();
    let out = h
        .brain()
        .env("PATH", stub.path())
        .env("HOME", home.path())
        .args(["connect", "claude-code", "--as=-chief"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = fs::read_to_string(stub.path().join("calls")).unwrap();
    let before_the_plugin = calls.lines().take_while(|l| *l != "plugin").last();
    assert_eq!(before_the_plugin, Some("--as=-chief"));
}

/// An installed brain from before `hook` exits 2 on it, which Claude Code reads as "block".
#[test]
fn connect_refuses_to_plug_in_a_brain_that_knows_no_hook() {
    let h = Home::new();
    let exe = install(&h);
    script(&exe, "[ \"$1\" = hook ] && exit 2");
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    let out = connect(&h, stub.path(), home.path());
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("hooks are not connected"), "{said}");
    assert!(said.contains("brain update"), "{said}");
    assert!(!plugin(&h).exists());
}

/// The binary behind a written plugin can still be replaced by an older one.
#[test]
fn the_hook_command_exits_0_with_a_binary_that_knows_no_hook() {
    let h = Home::new();
    let exe = install(&h);
    let stub = stub_claude();
    let home = tempfile::tempdir().unwrap();
    assert!(connect(&h, stub.path(), home.path()).status.success());
    script(&exe, "exit 2");
    let ran = std::process::Command::new("/bin/sh")
        .args(["-c", &command(&h, "UserPromptSubmit")])
        .status()
        .unwrap();
    assert!(ran.success());
}
