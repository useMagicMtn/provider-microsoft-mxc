use anyhow::Result;
use magicmtn_provider_microsoft_mxc::MxcProvider;
use magicmtn_sdk::plugin::{Capabilities, SandboxOperation::*};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    magicmtn_provider_framework::run_sandbox(
        "microsoft-mxc",
        env!("CARGO_PKG_VERSION"),
        Capabilities::sandbox(vec![Create, Terminate, Exec, ExecBounded]),
        MxcProvider::from_env()?,
    )
    .await
}
