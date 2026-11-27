//! Microsoft MXC containment with persistent workspace/policy sessions.
//! Each exec creates a fresh contained process tree; no resident VM or daemon.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use magicmtn_sdk::plugin::{Capabilities, ErrorCode, ProviderError, SandboxOperation};
use magicmtn_sdk::{
    BoxFuture, ExecResult, ProviderSandbox, SandboxBackend, SandboxState, ScpDirection,
};
use mxc_sdk::v1::{self as mxc, ContainerRequest, Containment, WaitResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn capabilities() -> Capabilities {
    use SandboxOperation::*;
    Capabilities::sandbox(vec![Create, Status, Terminate, Exec, ExecBounded])
}

/// Trusted server-local template. Only the session workspace is writable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    #[serde(default)]
    pub readonly_paths: Vec<PathBuf>,
    #[serde(default)]
    pub denied_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    version: u32,
    platform: String,
    template: Template,
}

/// The state directory must be private to the MagicMtn server account.
#[derive(Debug, Clone)]
pub struct MxcProvider {
    root: Arc<OnceLock<PathBuf>>,
}

impl MxcProvider {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        private_dir(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        private_dir(&root.join("sessions"))?;
        private_dir(&root.join("locks"))?;
        Ok(Self {
            root: Arc::new(OnceLock::from(root)),
        })
    }

    pub fn from_env() -> Result<Self> {
        let root = if let Some(path) = std::env::var_os("MAGICMTN_MXC_STATE_DIR") {
            PathBuf::from(path)
        } else if cfg!(windows) {
            PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .context("LOCALAPPDATA is unset; set MAGICMTN_MXC_STATE_DIR")?,
            )
            .join("MagicMtn/mxc")
        } else {
            PathBuf::from(
                std::env::var_os("HOME").context("HOME is unset; set MAGICMTN_MXC_STATE_DIR")?,
            )
            .join(".local/share/magicmtn/mxc")
        };
        Self::new(root)
    }

    /// Defer environment and filesystem access until a workspace operation.
    /// Discovery and shutdown can run without state configuration or host tools.
    pub fn deferred_from_env() -> Self {
        Self {
            root: Arc::new(OnceLock::new()),
        }
    }

    fn root(&self) -> Result<&Path> {
        if self.root.get().is_none() {
            let initialized = Self::from_env()?;
            let root = initialized
                .root
                .get()
                .context("MXC state was not initialized")?;
            // Concurrent initializers select the same immutable process environment.
            let _ = self.root.set(root.clone());
        }
        self.root
            .get()
            .map(PathBuf::as_path)
            .context("MXC state was not initialized")
    }

    fn id(sandbox: &ProviderSandbox) -> Result<Uuid> {
        let value = sandbox
            .provider_id
            .strip_prefix("mxc-")
            .context("invalid MXC session ID")?;
        let id = Uuid::parse_str(value).context("invalid MXC session UUID")?;
        ensure!(
            !id.is_nil() && id.to_string() == value,
            "noncanonical MXC session ID"
        );
        Ok(id)
    }

    // Serialize operations even across separate provider processes. Lock files
    // remain as small tombstones so waiting processes never lock a new inode.
    fn lock(&self, id: Uuid, create: bool) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .open(self.root()?.join("locks").join(format!("{id}.lock")))
            .context("unknown MXC session")?;
        FileExt::lock_exclusive(&file)?;
        Ok(file)
    }

    fn session_dir(&self, id: Uuid) -> Result<PathBuf> {
        Ok(self.root()?.join("sessions").join(id.to_string()))
    }

    // Unknown IDs are absent only when both the lock and session are absent.
    // Other filesystem errors and orphaned session directories remain errors.
    fn lock_existing(&self, id: Uuid) -> Result<Option<File>> {
        match self.lock(id, false) {
            Ok(lock) => Ok(Some(lock)),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                match fs::symlink_metadata(self.session_dir(id)?) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error.into()),
                    Ok(_) => bail!("MXC session lock is missing"),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn load_session(&self, id: Uuid) -> Result<(Session, PathBuf)> {
        let dir = self.session_dir(id)?;
        ensure!(
            fs::symlink_metadata(&dir)?.is_dir(),
            "MXC session identity changed"
        );
        let session: Session = serde_json::from_slice(&read_small(&dir.join("policy.json"))?)?;
        ensure!(
            session.version == 1 && session.platform == std::env::consts::OS,
            "incompatible MXC session state"
        );
        let workspace = dir.join("workspace").canonicalize()?;
        ensure!(
            workspace == dir.join("workspace"),
            "workspace identity changed"
        );
        Ok((session, workspace))
    }

    fn status_sync(&self, sandbox: &ProviderSandbox) -> Result<SandboxState> {
        let id = Self::id(sandbox)?;
        let Some(_lock) = self.lock_existing(id)? else {
            return Ok(SandboxState::Missing);
        };
        match fs::symlink_metadata(self.session_dir(id)?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SandboxState::Missing),
            Err(error) => Err(error.into()),
            Ok(_) => {
                self.load_session(id)?;
                // Running denotes an available workspace session; commands run separately.
                Ok(SandboxState::Running)
            }
        }
    }

    fn create_sync(&self, template: &str) -> Result<ProviderSandbox> {
        if template.starts_with("image:") {
            return Err(ProviderError {
                code: ErrorCode::UnsupportedOperation,
                retryable: false,
                message: "MXC process backends do not support OCI images; supply a JSON policy template path".into(),
            }.into());
        }
        let bytes = read_small(Path::new(template))?;
        let mut template: Template = serde_json::from_slice(&bytes)
            .context("MXC requires a JSON policy template, not a Dockerfile or OCI image")?;
        for path in template
            .readonly_paths
            .iter_mut()
            .chain(template.denied_paths.iter_mut())
        {
            ensure!(path.is_absolute(), "policy paths must be absolute");
            *path = path.canonicalize().context("policy path does not exist")?;
            ensure!(
                !self.root()?.starts_with(&*path) && !path.starts_with(self.root()?),
                "policy paths must not overlap provider state"
            );
        }
        let backend = backend()?;
        ensure!(
            mxc::available_backends()
                .iter()
                .any(|b| b.backend == backend),
            "MXC backend {backend} is unavailable; check platform requirements"
        );
        let id = Uuid::new_v4();
        let _lock = self.lock(id, true)?;
        let dir = self.session_dir(id)?;
        private_dir(&dir)?;
        let result = (|| {
            private_dir(&dir.join("workspace"))?;
            let session = Session {
                version: 1,
                platform: std::env::consts::OS.into(),
                template,
            };
            fs::write(dir.join("policy.json"), serde_json::to_vec(&session)?)?;
            Ok(ProviderSandbox {
                provider_id: format!("mxc-{id}"),
                status: Some("ready".into()),
                create_time_ms: Some(
                    SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64
                ),
                metadata: Some(
                    json!({"provider":"microsoft-mxc", "backend":backend, "mxc_version":"1.0.0", "workspace":dir.join("workspace"), "lifecycle":"workspace-session", "network":"deny"}),
                ),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&dir);
        }
        result
    }

    fn exec_sync(
        &self,
        sandbox: &ProviderSandbox,
        command: &str,
        args: &[String],
        timeout_ms: u64,
        limit: usize,
    ) -> Result<ExecResult> {
        ensure!(
            (1..=30_000).contains(&timeout_ms) && (1..=16_384).contains(&limit),
            "invalid exec limits"
        );
        ensure!(
            !command.is_empty()
                && command.len() <= 256
                && !command.starts_with('-')
                && !command.contains('\0'),
            "invalid program"
        );
        ensure!(
            args.len() <= 32 && args.iter().all(|a| a.len() <= 2048 && !a.contains('\0')),
            "invalid arguments"
        );
        let id = Self::id(sandbox)?;
        let _lock = self.lock(id, false)?;
        let (session, workspace) = self.load_session(id)?;
        let mut request = ContainerRequest::new(command_line(command, args, cfg!(windows)));
        request.containment = match backend()? {
            "seatbelt" => Containment::Seatbelt(Default::default()),
            "bubblewrap" => Containment::Bubblewrap,
            "processcontainer" => Containment::ProcessContainer(Default::default()),
            _ => unreachable!(),
        };
        request.working_directory = Some(workspace.to_string_lossy().into_owned());
        let mut environment = vec![
            ("HOME".into(), workspace.to_string_lossy().into_owned()),
            ("TMPDIR".into(), workspace.to_string_lossy().into_owned()),
            ("TEMP".into(), workspace.to_string_lossy().into_owned()),
            ("TMP".into(), workspace.to_string_lossy().into_owned()),
        ];
        if cfg!(windows) {
            let system =
                PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is unset")?);
            environment.extend([
                ("SystemRoot".into(), system.to_string_lossy().into_owned()),
                ("WINDIR".into(), system.to_string_lossy().into_owned()),
                (
                    "PATH".into(),
                    system.join("System32").to_string_lossy().into_owned(),
                ),
                (
                    "USERPROFILE".into(),
                    workspace.to_string_lossy().into_owned(),
                ),
                ("APPDATA".into(), workspace.to_string_lossy().into_owned()),
                (
                    "LOCALAPPDATA".into(),
                    workspace.to_string_lossy().into_owned(),
                ),
            ]);
        } else {
            environment.push(("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()));
        }
        request.environment = Some(environment);
        request.timeout_ms = Some(timeout_ms as u32);
        request.filesystem = Some(mxc::FilesystemPolicy {
            readwrite_paths: vec![workspace.to_string_lossy().into_owned()],
            readonly_paths: session
                .template
                .readonly_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            denied_paths: session
                .template
                .denied_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            clear_policy_on_exit: None,
        });
        request.network = Some(mxc::NetworkPolicy {
            egress: Some(mxc::NetworkEgressPolicy {
                default: Some(mxc::NetworkAction::Deny),
                ..Default::default()
            }),
            ingress: Some(mxc::NetworkIngressPolicy {
                default: Some(mxc::NetworkAction::Deny),
                host_loopback: Some(mxc::NetworkAction::Deny),
            }),
            ..Default::default()
        });
        request.ui = Some(mxc::UiPolicy::default());
        let start = Instant::now();
        let mut process =
            mxc::spawn(request, Default::default()).context("MXC contained launch failed")?;
        drop(process.take_stdin()); // Workloads cannot consume the plugin protocol.
        let stdout = process.take_stdout().context("MXC stdout missing")?;
        let stderr = process.take_stderr().context("MXC stderr missing")?;
        let out_closer = process.stdout_closer();
        let err_closer = process.stderr_closer();
        let (out, err, outcome) = std::thread::scope(|scope| {
            let out = scope.spawn(move || capture(stdout, limit));
            let err = scope.spawn(move || capture(stderr, limit));
            let outcome = process.wait();
            if outcome.is_err() {
                let _ = process.kill();
            }
            // Release backend-owned write ends and terminate remaining members
            // of the contained process tree before joining the stream readers.
            drop(process);
            if !matches!(outcome, Ok(WaitResult::Exited(_))) {
                if let Some(c) = out_closer {
                    c.close();
                }
                if let Some(c) = err_closer {
                    c.close();
                }
            }
            (out.join(), err.join(), outcome)
        });
        let (stdout, a) = out.map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
        let (stderr, b) = err.map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;
        let (exit_code, timed_out) = match outcome? {
            WaitResult::Exited(code) => (Some(code), false),
            WaitResult::TimedOut => (None, true),
        };
        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
            elapsed_ms: start.elapsed().as_millis() as u64,
            timed_out,
            truncated: a || b,
        })
    }
}

fn unsupported_image() -> anyhow::Error {
    ProviderError {
        code: ErrorCode::UnsupportedOperation,
        retryable: false,
        message: "MXC uses host-native JSON policy templates".into(),
    }
    .into()
}

fn backend() -> Result<&'static str> {
    match std::env::consts::OS {
        "macos" => Ok("seatbelt"),
        "linux" => Ok("bubblewrap"),
        "windows" => Ok("processcontainer"),
        _ => bail!("MXC provider supports macOS, Linux, and Windows"),
    }
}

// Preserve actionable errors across the framework's intentionally generic
// fallback. No error is marked retryable: an execution may have taken effect.
fn rpc_error(error: anyhow::Error) -> anyhow::Error {
    if error.is::<ProviderError>() {
        return error;
    }
    let code = if error.is::<mxc::Error>() {
        ErrorCode::UpstreamUnavailable
    } else {
        ErrorCode::Internal
    };
    ProviderError {
        code,
        retryable: false,
        message: format!("{error:#}"),
    }
    .into()
}

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn read_small(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("cannot read {}", path.display()))?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65_536, "policy template exceeds 64 KiB");
    Ok(bytes)
}

// MXC consumes a POSIX shell command on Unix and a CreateProcess command
// line on Windows. Quoting each argv item prevents accidental shell syntax.
fn command_line(command: &str, args: &[String], windows: bool) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(|s| {
            if windows {
                windows_quote(s)
            } else {
                format!("'{}'", s.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn windows_quote(s: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in s.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        result.push_str(&"\\".repeat(if c == '"' { slashes * 2 + 1 } else { slashes }));
        result.push(c);
        slashes = 0;
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}

fn capture(mut reader: impl Read, limit: usize) -> Result<(String, bool)> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let keep = count.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..keep]);
        truncated |= count > keep;
    }
    // A lossy replacement can expand invalid UTF-8. Cap the encoded response too.
    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    let mut end = output.len().min(limit);
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    truncated |= end < output.len();
    output.truncate(end);
    Ok((output, truncated))
}

impl SandboxBackend for MxcProvider {
    fn status<'a>(&'a self, sandbox: &'a ProviderSandbox) -> BoxFuture<'a, SandboxState> {
        let provider = self.clone();
        let sandbox = sandbox.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || provider.status_sync(&sandbox))
                .await?
                .map_err(rpc_error)
        })
    }

    fn prepare_image<'a>(
        &'a self,
        _: &'a std::collections::BTreeMap<String, magicmtn_sdk::image_registry::Image>,
    ) -> BoxFuture<'a, magicmtn_sdk::image_registry::PreparedImage> {
        Box::pin(async { Err(unsupported_image()) })
    }

    fn create_from_image<'a>(&'a self, _: &'a Path) -> BoxFuture<'a, ProviderSandbox> {
        Box::pin(async { Err(unsupported_image()) })
    }

    fn create<'a>(&'a self, template: &'a str) -> BoxFuture<'a, ProviderSandbox> {
        let provider = self.clone();
        let template = template.to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || provider.create_sync(&template))
                .await?
                .map_err(rpc_error)
        })
    }
    fn terminate<'a>(&'a self, sandbox: &'a ProviderSandbox) -> BoxFuture<'a, ()> {
        let provider = self.clone();
        let sandbox = sandbox.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let id = Self::id(&sandbox)?;
                let Some(_lock) = provider.lock_existing(id)? else {
                    return Ok(());
                };
                match fs::remove_dir_all(provider.session_dir(id)?) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.into()),
                }
            })
            .await?
            .map_err(rpc_error)
        })
    }
    fn exec<'a>(
        &'a self,
        sandbox: &'a ProviderSandbox,
        command: &'a str,
        args: &'a [String],
    ) -> BoxFuture<'a, String> {
        Box::pin(async move {
            let result = self
                .exec_bounded(sandbox, command, args, 30_000, 16_384)
                .await?;
            ensure!(
                !result.timed_out,
                "MXC command timed out; do not replay automatically"
            );
            ensure!(
                result.exit_code == Some(0),
                "MXC command exited with {:?}: {}",
                result.exit_code,
                result.stderr
            );
            Ok(result.stdout)
        })
    }
    fn exec_bounded<'a>(
        &'a self,
        sandbox: &'a ProviderSandbox,
        command: &'a str,
        args: &'a [String],
        timeout_ms: u64,
        max_output_bytes: usize,
    ) -> BoxFuture<'a, ExecResult> {
        let provider = self.clone();
        let sandbox = sandbox.clone();
        let command = command.to_owned();
        let args = args.to_vec();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                provider.exec_sync(&sandbox, &command, &args, timeout_ms, max_output_bytes)
            })
            .await?
            .map_err(rpc_error)
        })
    }
    fn ssh<'a>(&'a self, _: &'a ProviderSandbox) -> BoxFuture<'a, ()> {
        Box::pin(async { bail!("interactive MXC shells are unsupported") })
    }
    fn scp<'a>(
        &'a self,
        _: &'a ProviderSandbox,
        _: &'a Path,
        _: &'a Path,
        _: ScpDirection,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async { bail!("MXC file transfer is unsupported") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn argv_quoting() {
        assert_eq!(
            command_line("echo", &["a'; $(touch /tmp/oops)".into(), "".into()], false),
            "'echo' 'a'\\''; $(touch /tmp/oops)' ''"
        );
        assert_eq!(windows_quote("a\\\"b\\"), "\"a\\\\\\\"b\\\\\"");
        assert_eq!(windows_quote(""), "\"\"");
        assert_eq!(windows_quote("a & b"), "\"a & b\"");
    }
    #[test]
    fn output_is_bounded_and_drained() {
        let mut input = std::io::Cursor::new(vec![b'x'; 100_000]);
        let (output, truncated) = capture(&mut input, 10).unwrap();
        assert_eq!(output, "xxxxxxxxxx");
        assert!(truncated);
        assert_eq!(input.position(), 100_000);
        assert!(capture(&[0xff; 10][..], 10).unwrap().0.len() <= 10);
        assert_eq!(capture(&b"ok"[..], 10).unwrap(), ("ok".into(), false));
    }
    #[test]
    fn rejects_invalid_templates_and_ids_without_launching() {
        let tmp = tempfile::tempdir().unwrap();
        let p = MxcProvider::new(tmp.path().join("state")).unwrap();
        assert!(p.create_sync("image:alpine|bad").is_err());
        let template = tmp.path().join("template.json");
        for value in [
            json!({"network":"allow"}),
            json!({"readonly_paths":["relative"]}),
            json!({"readonly_paths":[tmp.path()]}),
        ] {
            fs::write(&template, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(p.create_sync(template.to_str().unwrap()).is_err());
        }
        for id in [
            "../other",
            "mxc-../../x",
            "mxc-00000000-0000-0000-0000-000000000000",
        ] {
            assert!(
                MxcProvider::id(&ProviderSandbox {
                    provider_id: id.into(),
                    status: None,
                    create_time_ms: None,
                    metadata: None
                })
                .is_err()
            );
        }
    }
    fn saved_session(provider: &MxcProvider) -> ProviderSandbox {
        let id = Uuid::new_v4();
        let _lock = provider.lock(id, true).unwrap();
        let dir = provider.session_dir(id).unwrap();
        private_dir(&dir.join("workspace")).unwrap();
        fs::write(
            dir.join("policy.json"),
            serde_json::to_vec(&Session {
                version: 1,
                platform: std::env::consts::OS.into(),
                template: Template::default(),
            })
            .unwrap(),
        )
        .unwrap();
        ProviderSandbox {
            provider_id: format!("mxc-{id}"),
            status: None,
            create_time_ms: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn status_and_removal_use_durable_state_across_provider_instances() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MxcProvider::new(tmp.path().join("state")).unwrap();
        let sandbox = saved_session(&provider);
        assert_eq!(
            provider.status(&sandbox).await.unwrap(),
            SandboxState::Running
        );
        let restarted = MxcProvider::new(tmp.path().join("state")).unwrap();
        assert_eq!(
            restarted.status(&sandbox).await.unwrap(),
            SandboxState::Running
        );
        restarted.terminate(&sandbox).await.unwrap();
        assert_eq!(
            provider.status(&sandbox).await.unwrap(),
            SandboxState::Missing
        );
        provider.terminate(&sandbox).await.unwrap();
        let unknown = ProviderSandbox {
            provider_id: format!("mxc-{}", Uuid::new_v4()),
            ..sandbox
        };
        assert_eq!(
            provider.status(&unknown).await.unwrap(),
            SandboxState::Missing
        );
        provider.terminate(&unknown).await.unwrap();
        assert_eq!(
            fs::read_dir(provider.root().unwrap().join("locks"))
                .unwrap()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn damaged_state_is_an_error_instead_of_a_missing_session() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MxcProvider::new(tmp.path().join("state")).unwrap();
        let sandbox = saved_session(&provider);
        let id = MxcProvider::id(&sandbox).unwrap();
        let dir = provider.session_dir(id).unwrap();
        for body in [
            "invalid".to_owned(),
            json!({"version":2,"platform":std::env::consts::OS,"template":{}}).to_string(),
            json!({"version":1,"platform":"another-host","template":{}}).to_string(),
        ] {
            fs::write(dir.join("policy.json"), body).unwrap();
            assert!(provider.status(&sandbox).await.is_err());
        }
        fs::remove_file(dir.join("policy.json")).unwrap();
        assert!(provider.status(&sandbox).await.is_err());
        fs::remove_file(
            provider
                .root()
                .unwrap()
                .join("locks")
                .join(format!("{id}.lock")),
        )
        .unwrap();
        assert!(provider.status(&sandbox).await.is_err());
        assert!(provider.terminate(&sandbox).await.is_err());
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn image_contracts_fail_explicitly_without_initializing_state() {
        let provider = MxcProvider::deferred_from_env();
        let error = provider
            .prepare_image(&Default::default())
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ProviderError>().unwrap().code,
            ErrorCode::UnsupportedOperation
        );
        let error = provider
            .create_from_image(Path::new("unused"))
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ProviderError>().unwrap().code,
            ErrorCode::UnsupportedOperation
        );
        assert!(provider.root.get().is_none());
    }
    #[tokio::test]
    async fn a_missing_workspace_does_not_make_an_existing_session_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MxcProvider::new(tmp.path().join("state")).unwrap();
        let sandbox = saved_session(&provider);
        let dir = provider
            .session_dir(MxcProvider::id(&sandbox).unwrap())
            .unwrap();
        fs::remove_dir(dir.join("workspace")).unwrap();
        assert!(provider.status(&sandbox).await.is_err());
        assert!(dir.join("policy.json").is_file());
        provider.terminate(&sandbox).await.unwrap();
        assert_eq!(
            provider.status(&sandbox).await.unwrap(),
            SandboxState::Missing
        );
    }
}
