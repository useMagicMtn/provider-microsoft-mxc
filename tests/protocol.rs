#![cfg(feature = "plugin")]
use magicmtn_sdk::plugin::{
    ErrorCode, Operation, Outcome, PROTOCOL_VERSION, Request, Response, SandboxOperation, Value,
};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

#[test]
fn persistent_rpc_describe_errors_and_shutdown() {
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_magicmtn-provider-microsoft-mxc"))
        .env("MAGICMTN_MXC_STATE_DIR", tmp.path().join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id, operation| {
        serde_json::to_writer(
            &mut input,
            &Request {
                id,
                protocol_version: PROTOCOL_VERSION,
                provider: "microsoft-mxc".into(),
                operation,
            },
        )
        .unwrap();
        writeln!(input).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let reply: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(reply.id, id);
        assert_eq!(reply.provider, "microsoft-mxc");
        assert_eq!(reply.protocol_version, PROTOCOL_VERSION);
        reply.outcome
    };
    for id in 1..=2 {
        let Outcome::Ok(Value::Description(manifest)) = call(id, Operation::Describe) else {
            panic!("missing manifest")
        };
        assert_eq!(
            manifest.capabilities.sandbox_operations,
            vec![
                SandboxOperation::Create,
                SandboxOperation::Status,
                SandboxOperation::Terminate,
                SandboxOperation::Exec,
                SandboxOperation::ExecBounded
            ]
        );
    }
    assert!(matches!(
        call(
            3,
            Operation::SandboxCreate {
                template: "image:alpine|bad".into(),
                ttl_seconds: None,
            }
        ),
        Outcome::Error(_)
    ));
    let sandbox = magicmtn_sdk::ProviderSandbox {
        provider_id: "bad".into(),
        status: None,
        create_time_ms: None,
        metadata: None,
    };
    let Outcome::Error(error) = call(4, Operation::SandboxSsh { sandbox }) else {
        panic!("unsupported SSH accepted")
    };
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(matches!(
        call(5, Operation::Shutdown),
        Outcome::Ok(Value::Done)
    ));
    drop(input);
    assert!(child.wait().unwrap().success());
    let mut trailing = String::new();
    output.read_line(&mut trailing).unwrap();
    assert!(trailing.is_empty());
}

#[test]
fn discovery_and_shutdown_need_no_environment_or_writable_state() {
    for blocked_state in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        std::fs::write(&state, "a file, not a state directory").unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_magicmtn-provider-microsoft-mxc"));
        command.env_clear();
        if blocked_state {
            command.env("MAGICMTN_MXC_STATE_DIR", &state);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for (i, operation) in [Operation::Describe, Operation::Shutdown]
            .into_iter()
            .enumerate()
        {
            serde_json::to_writer(
                &mut input,
                &Request {
                    id: i as u64 + 1,
                    protocol_version: PROTOCOL_VERSION,
                    provider: "microsoft-mxc".into(),
                    operation,
                },
            )
            .unwrap();
            writeln!(input).unwrap();
        }
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let responses: Vec<Response> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 2);
        let Outcome::Ok(Value::Description(manifest)) = &responses[0].outcome else {
            panic!("missing manifest")
        };
        assert!(manifest.credential.is_none());
        assert!(manifest.capabilities.setup.is_none());
        assert!(
            !manifest
                .capabilities
                .sandbox_operations
                .contains(&SandboxOperation::PrepareImage)
        );
        assert!(matches!(responses[1].outcome, Outcome::Ok(Value::Done)));
        assert_eq!(
            std::fs::read_to_string(&state).unwrap(),
            "a file, not a state directory"
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
}

#[test]
fn persistent_rpc_reports_saved_sessions_and_idempotent_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("state");
    let id = uuid::Uuid::new_v4();
    let dir = root.join("sessions").join(id.to_string());
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    std::fs::create_dir_all(root.join("locks")).unwrap();
    std::fs::write(root.join("locks").join(format!("{id}.lock")), "").unwrap();
    std::fs::write(
        dir.join("policy.json"),
        serde_json::to_vec(&serde_json::json!({
            "version":1,"platform":std::env::consts::OS,"template":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let sandbox = magicmtn_sdk::ProviderSandbox {
        provider_id: format!("mxc-{id}"),
        status: Some("missing".into()),
        create_time_ms: None,
        metadata: Some(serde_json::json!({"workspace":"/untrusted"})),
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_magicmtn-provider-microsoft-mxc"))
        .env("MAGICMTN_MXC_STATE_DIR", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for (i, operation) in [
        Operation::SandboxStatus {
            sandbox: sandbox.clone(),
        },
        Operation::SandboxTerminate {
            sandbox: sandbox.clone(),
        },
        Operation::SandboxStatus {
            sandbox: sandbox.clone(),
        },
        Operation::SandboxTerminate {
            sandbox: sandbox.clone(),
        },
        Operation::Shutdown,
    ]
    .into_iter()
    .enumerate()
    {
        serde_json::to_writer(
            &mut input,
            &Request {
                id: i as u64 + 1,
                protocol_version: PROTOCOL_VERSION,
                provider: "microsoft-mxc".into(),
                operation,
            },
        )
        .unwrap();
        writeln!(input).unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses: Vec<Response> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 5);
    assert!(matches!(
        responses[0].outcome,
        Outcome::Ok(Value::SandboxStatus(magicmtn_sdk::SandboxState::Running))
    ));
    assert!(matches!(responses[1].outcome, Outcome::Ok(Value::Done)));
    assert!(matches!(
        responses[2].outcome,
        Outcome::Ok(Value::SandboxStatus(magicmtn_sdk::SandboxState::Missing))
    ));
    assert!(matches!(responses[3].outcome, Outcome::Ok(Value::Done)));
    assert!(!dir.exists());
    assert!(root.join("locks").join(format!("{id}.lock")).is_file());
}

#[tokio::test]
async fn current_sdk_host_accepts_status_and_gates_image_operations() {
    use magicmtn_provider_framework::host::Client;
    use std::{path::Path, time::Duration};
    let client = Client::start(
        Path::new(env!("CARGO_BIN_EXE_magicmtn-provider-microsoft-mxc")),
        "microsoft-mxc",
    )
    .await
    .unwrap();
    assert!(
        client
            .manifest
            .capabilities
            .sandbox_operations
            .contains(&SandboxOperation::Status)
    );
    assert!(client.manifest.credential.is_none());
    let error = client
        .call(
            Operation::SandboxPrepareImage {
                images: Default::default(),
            },
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not support"));
    client.shutdown().await;
}
