//! Process-level smoke test for the shipped agent server executable (#463).

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agent_client::AgentClient;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

struct ServerProcess {
    child: Child,
}

impl ServerProcess {
    fn start(port: u16, rom_path: &std::path::Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_spec-chum-agent"))
            .args([
                "--model",
                "48k",
                "--port",
                &port.to_string(),
                "--insecure",
                "--rom",
            ])
            .arg(rom_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("failed to launch spec-chum-agent: {error}"));
        Self { child }
    }

    fn from_child(child: Child) -> Self {
        Self { child }
    }

    fn stop(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout);
        }
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        format!("stdout:\n{stdout}\nstderr:\n{stderr}")
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct FixtureRom(PathBuf);

impl FixtureRom {
    fn create() -> Self {
        let path = std::env::temp_dir().join(format!(
            "spec-chum-agent-rom-{}-{}.rom",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        std::fs::write(&path, vec![0; 16 * 1024]).expect("write deterministic ROM fixture");
        Self(path)
    }
}

impl Drop for FixtureRom {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn isolated_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("allocate isolated loopback port");
    listener.local_addr().expect("read allocated port").port()
}

fn client(port: u16) -> AgentClient {
    AgentClient::with_timeout(
        &format!("http://127.0.0.1:{port}"),
        None,
        Duration::from_millis(250),
    )
}

fn wait_until_ready(
    server: &mut ServerProcess,
    client: &AgentClient,
) -> Result<serde_json::Value, String> {
    wait_until_ready_with_timeout(server, client, STARTUP_TIMEOUT)
}

fn wait_until_ready_with_timeout(
    server: &mut ServerProcess,
    client: &AgentClient,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = server.child.try_wait().map_err(|e| e.to_string())? {
            let output = server.stop();
            return Err(format!("server exited during startup ({status}); {output}"));
        }
        if let Ok(response) = client.health() {
            if response["ok"] == true {
                return Ok(response);
            }
        }
        if Instant::now() >= deadline {
            let output = server.stop();
            return Err(format!(
                "server did not become ready within {timeout:?}; {output}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "launched as an unresponsive subprocess by startup-timeout test"]
fn process_harness_non_server_child() {
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[test]
fn executable_starts_serves_inspect_and_is_cleaned_up() {
    let fixture = FixtureRom::create();
    let port = isolated_port();
    let mut server = ServerProcess::start(port, &fixture.0);
    let client = client(port);
    let health = wait_until_ready(&mut server, &client).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        health["has_machine"] == true,
        "unexpected health response: {health}"
    );

    let inspect = client.inspect_json().unwrap_or_else(|error| {
        let diagnostics = server.stop();
        panic!("GET /v1/inspect failed: {error}; {diagnostics}");
    });
    let inspect: serde_json::Value = serde_json::from_str(&inspect).expect("inspect JSON");
    assert!(
        inspect["model"] == "48k" && inspect["pc"] == 0,
        "inspect did not return expected fixture state (48k, PC=0): {inspect}"
    );

    let diagnostics = server.stop();
    assert!(
        client.health().is_err(),
        "server still accepts connections after cleanup; {diagnostics}"
    );
}

#[test]
fn startup_failure_reports_child_diagnostics() {
    let fixture = FixtureRom::create();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve collision port");
    let port = listener.local_addr().expect("read collision port").port();
    let mut server = ServerProcess::start(port, &fixture.0);
    let error =
        wait_until_ready(&mut server, &client(port)).expect_err("occupied port must fail startup");
    assert!(
        error.contains("server exited during startup"),
        "missing startup failure context: {error}"
    );
    assert!(
        error
            .to_ascii_lowercase()
            .contains("address already in use"),
        "missing bind diagnostic: {error}"
    );
}

#[test]
fn startup_timeout_reports_diagnostics_and_reaps_child() {
    let child = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "process_harness_non_server_child",
            "--ignored",
            "--nocapture",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch unresponsive child process");
    let mut server = ServerProcess::from_child(child);
    let error = wait_until_ready_with_timeout(
        &mut server,
        &client(isolated_port()),
        Duration::from_millis(150),
    )
    .expect_err("non-server child must time out");

    assert!(
        error.contains("server did not become ready within 150ms"),
        "missing timeout diagnostic: {error}"
    );
    assert!(
        error.contains("stdout:") && error.contains("stderr:"),
        "missing child output diagnostics: {error}"
    );
    assert!(
        server
            .child
            .try_wait()
            .expect("check child process state")
            .is_some(),
        "timeout path did not kill and reap the child"
    );
}
