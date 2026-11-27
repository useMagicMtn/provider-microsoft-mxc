use anyhow::Result;
use magicmtn_provider_microsoft_mxc::{MxcProvider, capabilities};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    magicmtn_provider_framework::run_sandbox(
        "microsoft-mxc",
        env!("CARGO_PKG_VERSION"),
        capabilities(),
        MxcProvider::deferred_from_env(),
    )
    .await
}
