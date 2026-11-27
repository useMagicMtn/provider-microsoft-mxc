//! Create and execute a host-native workspace using a local JSON policy.
use anyhow::{Context, Result};
use magicmtn_provider_microsoft_mxc::MxcProvider;
use magicmtn_sdk::{SandboxBackend, SandboxState};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let policy = std::env::args()
        .nth(1)
        .context("usage: workspace /absolute/path/to/policy.json")?;
    let provider = MxcProvider::from_env()?;
    let sandbox = provider.create(&policy).await?;
    let result = async {
        anyhow::ensure!(
            provider.status(&sandbox).await? == SandboxState::Running,
            "workspace is unavailable"
        );
        #[cfg(unix)]
        let (program, args) = ("/bin/echo", vec!["Hello from MXC".into()]);
        #[cfg(windows)]
        let (program, args) = (
            "cmd.exe",
            vec!["/d".into(), "/c".into(), "echo Hello from MXC".into()],
        );
        provider.exec(&sandbox, program, &args).await
    }
    .await;
    provider.terminate(&sandbox).await?;
    print!("{}", result?);
    Ok(())
}
