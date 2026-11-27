use magicmtn_provider_microsoft_mxc::MxcProvider;
use magicmtn_sdk::{ProviderSandbox, SandboxBackend, SandboxState};
use serde_json::json;

async fn session() -> (tempfile::TempDir, MxcProvider, ProviderSandbox) {
    let tmp = tempfile::tempdir().unwrap();
    let provider = MxcProvider::new(tmp.path().join("state")).unwrap();
    let policy = tmp.path().join("policy.json");
    std::fs::write(&policy, "{}").unwrap();
    let sandbox = provider.create(policy.to_str().unwrap()).await.unwrap();
    (tmp, provider, sandbox)
}

#[tokio::test]
#[ignore = "requires a working native MXC backend"]
async fn workspace_survives_restart_and_termination_revokes_execution() {
    let (tmp, provider, mut sandbox) = session().await;
    #[cfg(unix)]
    let (program, args) = (
        "/bin/sh",
        vec![
            "-c".into(),
            "printf persisted > marker; printf hello".into(),
        ],
    );
    #[cfg(windows)]
    let (program, args) = (
        "cmd.exe",
        vec![
            "/d".into(),
            "/c".into(),
            "echo persisted>marker & echo hello".into(),
        ],
    );
    assert!(
        provider
            .exec(&sandbox, program, &args)
            .await
            .unwrap()
            .contains("hello")
    );
    assert_eq!(
        provider.status(&sandbox).await.unwrap(),
        SandboxState::Running
    );
    // Neither editable metadata nor the original template controls later exec.
    sandbox.metadata = Some(json!({"workspace":"/", "network":"allow"}));
    std::fs::write(tmp.path().join("policy.json"), "invalid").unwrap();
    let restarted = MxcProvider::new(tmp.path().join("state")).unwrap();
    #[cfg(unix)]
    let (program, args) = ("/bin/cat", vec!["marker".into()]);
    #[cfg(windows)]
    let (program, args) = (
        "cmd.exe",
        vec!["/d".into(), "/c".into(), "type marker".into()],
    );
    assert!(
        restarted
            .exec(&sandbox, program, &args)
            .await
            .unwrap()
            .contains("persisted")
    );
    assert_eq!(
        restarted.status(&sandbox).await.unwrap(),
        SandboxState::Running
    );
    restarted.terminate(&sandbox).await.unwrap();
    assert_eq!(
        restarted.status(&sandbox).await.unwrap(),
        SandboxState::Missing
    );
    restarted.terminate(&sandbox).await.unwrap();
    assert!(restarted.exec(&sandbox, program, &args).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires a working native MXC backend"]
async fn bounded_streams_timeout_quoting_and_filesystem_isolation() {
    let (tmp, provider, sandbox) = session().await;
    let script = "i=0; while [ $i -lt 5000 ]; do printf 1234567890; printf abcdefghij >&2; i=$((i+1)); done; exit 7";
    let result = provider
        .exec_bounded(
            &sandbox,
            "/bin/sh",
            &["-c".into(), script.into()],
            10_000,
            64,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(7));
    assert!(result.truncated);
    assert_eq!(result.stdout.len(), 64);
    assert_eq!(result.stderr.len(), 64);
    let data = "hello'; $(touch injected); \" &\n";
    assert_eq!(
        provider
            .exec(&sandbox, "/usr/bin/printf", &["%s".into(), data.into()])
            .await
            .unwrap(),
        data
    );
    let workspace = std::path::PathBuf::from(
        sandbox.metadata.as_ref().unwrap()["workspace"]
            .as_str()
            .unwrap(),
    );
    assert!(!workspace.join("injected").exists());
    let secret = tmp.path().join("secret");
    std::fs::write(&secret, "private").unwrap();
    let result = provider
        .exec_bounded(
            &sandbox,
            "/bin/cat",
            &[secret.to_str().unwrap().into()],
            5000,
            512,
        )
        .await
        .unwrap();
    assert_ne!(result.exit_code, Some(0));
    assert!(!result.stdout.contains("private"));
    let result = provider
        .exec_bounded(
            &sandbox,
            "/bin/sh",
            &["-c".into(), "printf overwrite > ../policy.json".into()],
            5000,
            512,
        )
        .await
        .unwrap();
    assert_ne!(result.exit_code, Some(0));
    let result = provider
        .exec_bounded(
            &sandbox,
            "/bin/sh",
            &["-c".into(), "printf before; sleep 20".into()],
            150,
            512,
        )
        .await
        .unwrap();
    assert!(result.timed_out);
    assert_eq!(result.exit_code, None);
    assert!(result.elapsed_ms < 8000);
    assert!(result.stdout.contains("before"));
    // A background child holding pipe ends must not keep capture alive.
    let result = provider
        .exec_bounded(
            &sandbox,
            "/bin/sh",
            &["-c".into(), "sleep 20 & printf done".into()],
            500,
            512,
        )
        .await
        .unwrap();
    assert!(result.elapsed_ms < 8000);
    provider.terminate(&sandbox).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires a working native MXC backend"]
async fn network_is_denied() {
    let (_tmp, provider, sandbox) = session().await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let result = provider
        .exec_bounded(
            &sandbox,
            "/usr/bin/curl",
            &[
                "--noproxy".into(),
                "*".into(),
                "--max-time".into(),
                "1".into(),
                format!("http://127.0.0.1:{port}/"),
            ],
            3000,
            512,
        )
        .await
        .unwrap();
    assert_ne!(result.exit_code, Some(0));
    assert!(listener.accept().is_err());
    provider.terminate(&sandbox).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires a working native MXC backend"]
async fn explicit_readonly_and_denied_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("public"), "visible").unwrap();
    std::fs::write(source.join("private"), "secret").unwrap();
    let policy = tmp.path().join("template.json");
    std::fs::write(
        &policy,
        serde_json::to_vec(
            &json!({"readonly_paths":[source], "denied_paths":[source.join("private")]}),
        )
        .unwrap(),
    )
    .unwrap();
    let provider = MxcProvider::new(tmp.path().join("state")).unwrap();
    let sandbox = provider.create(policy.to_str().unwrap()).await.unwrap();
    assert_eq!(
        provider
            .exec(
                &sandbox,
                "/bin/cat",
                &[source.join("public").to_str().unwrap().into()]
            )
            .await
            .unwrap(),
        "visible"
    );
    assert!(
        provider
            .exec(
                &sandbox,
                "/bin/cat",
                &[source.join("private").to_str().unwrap().into()]
            )
            .await
            .is_err()
    );
    let result = provider
        .exec_bounded(
            &sandbox,
            "/usr/bin/touch",
            &[source.join("new").to_str().unwrap().into()],
            5000,
            512,
        )
        .await
        .unwrap();
    assert_ne!(result.exit_code, Some(0));
    assert!(!source.join("new").exists());
    provider.terminate(&sandbox).await.unwrap();
}
