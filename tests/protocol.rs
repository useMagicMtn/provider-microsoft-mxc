#![cfg(feature = "plugin")]
use magicmtn_sdk::plugin::{
    ErrorCode, Operation, Outcome, Request, Response, SandboxOperation, Value,
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
                protocol_version: 1,
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
