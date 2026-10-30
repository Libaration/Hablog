//! Runs `ObserverHandle` against a stand-in for `automation/host.mjs` that
//! speaks the same line protocol, so no game or Frida is needed. Needs Node.
use hablog::model::Direction;
use hablog::origins::observer::{ObserverConfig, ObserverEvent, ObserverHandle};
use std::path::PathBuf;
use std::time::Duration;

const STAND_IN: &str = r#"
import readline from 'node:readline';
const emit = event => process.stdout.write(JSON.stringify(event) + '\n');
emit({ type: 'status', state: 'companion_ready', protocol: 1 });
for await (const line of readline.createInterface({ input: process.stdin })) {
  const message = JSON.parse(line);
  if (message.command === 'start') {
    const { clientExecutablePath, modulePath, launch } = message.config;
    emit({ type: 'status', state: 'client_spawned', pid: 7 });
    emit({ type: 'status', state: 'observing', detail: `${clientExecutablePath}|${modulePath}|${launch}` });
    process.stdout.write('not json\n');
    emit({ type: 'crypto', operation: 'decrypt_payload', direction: 'server_to_client', kind: 'payload',
           sequence: '3', plaintext_side: 'output', input_hex: '5a5a', output_hex: '4042' });
    emit({ type: 'status', state: 'detached', reason: 'process-terminated' });
  } else if (message.command === 'stop') {
    process.exit(0);
  }
}
"#;

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[tokio::test]
async fn relays_the_companion_protocol_and_stops_it() {
    if !node_available() {
        eprintln!("skipped: Node.js is not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("hablog-observer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("host.mjs");
    std::fs::write(&script, STAND_IN).unwrap();

    let (mut handle, mut events) = ObserverHandle::start(ObserverConfig {
        node_exe: PathBuf::from("node"),
        host_script: script,
        client_exe: PathBuf::from("Hablog-Routed.exe"),
        module_path: PathBuf::from("BobbaXtra.x32"),
        launch: true,
        argv: Vec::new(),
    })
    .await
    .unwrap();

    let mut received = Vec::new();
    while received.len() < 5 {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("timed out")
            .expect("companion ended early");
        received.push(event);
    }
    assert!(
        matches!(&received[0], ObserverEvent::Status { state, .. } if state == "companion_ready")
    );
    assert!(matches!(received[1], ObserverEvent::Spawned));
    assert!(
        matches!(&received[2], ObserverEvent::Status { state, detail }
        if state == "observing" && detail == "Hablog-Routed.exe|BobbaXtra.x32|true")
    );
    match &received[3] {
        ObserverEvent::Crypto(op) => {
            assert_eq!(op.direction, Direction::ServerToClient);
            assert_eq!(op.sequence, "3");
            assert_eq!(op.plaintext_hex(), Some("4042"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(
        matches!(&received[4], ObserverEvent::Detached { reason } if reason == "process-terminated")
    );

    tokio::time::timeout(Duration::from_secs(10), handle.stop())
        .await
        .expect("stop timed out");
    std::fs::remove_dir_all(dir).unwrap();
}
