# Microsoft MXC provider usage

## Host and policy configuration

Set `MAGICMTN_MXC_STATE_DIR` in the environment of the MagicMtn server or provider runner to choose a stable state directory. Defaults are `$HOME/.local/share/magicmtn/mxc` on Unix and `%LOCALAPPDATA%\MagicMtn\mxc` on Windows. Unix directories are set to mode 0700. On Windows, use a directory with an ACL private to the server account.

Create a trusted server-local JSON policy file, for example:

```json
{
  "readonly_paths": ["/absolute/path/to/source"],
  "denied_paths": ["/absolute/path/to/source/private"]
}
```

Replace the paths with existing host files or directories. Both fields default to empty; [the example policy](examples/policy.json) uses those defaults. Paths inside the policy must be absolute. Creation resolves symlinks and rejects unknown fields, nonexistent paths, and paths that contain or sit inside provider state. Policy files are limited to 64 KiB. On Windows, escape backslashes in JSON, for example `"C:\\tools"`.

Pass the policy file's path as the template argument to `SandboxBackend::create`. Creation allocates an empty writable workspace and saves the resolved policy for that session. Later edits to the original file do not change existing sessions. Metadata identifies the backend and workspace for diagnostics; it does not grant access.

Include any extra tool directories, such as `/opt/homebrew`, in `readonly_paths` when needed, and invoke executables by their full paths if they are outside the sandbox's PATH. MXC also grants backend-specific baseline access to system and runtime files. Only the session workspace is writable; host source directories remain read-only.

## SDK workflow

Use the current public SDK pinned in [SDK_REVISION](SDK_REVISION). The executable speaks protocol v1 and advertises `create`, `status`, `terminate`, `exec`, and `exec_bounded`. Describe and shutdown work with an empty environment and perform no state initialization, backend detection, or sandbox launch. State is selected and initialized on the first workspace operation and shared by subsequent operations and library clones.

For a local Rust application, follow [CONTRIBUTING.md](CONTRIBUTING.md#development-setup) to configure the SDK paths, then run the workspace example:

```sh
cargo run --locked --example workspace -- /absolute/path/to/policy.json
```

The example creates a session, checks its status, executes a host-native command, and removes the session even if execution fails. For embedding, use `MxcProvider::new(private_state_directory)` or `MxcProvider::from_env()`. `MxcProvider::deferred_from_env()` selects state on the first workspace operation and supports state-free executable discovery.

An SDK host passes the local policy file path in `Operation::SandboxCreate` and retains the returned `ProviderSandbox` for later operations. Keep the provider's state directory stable across host restarts. `Operation::SandboxStatus` reads the saved policy and workspace under the session lock: `running` means the workspace session is available, and `missing` means it has been removed or its identifier is unknown. Malformed state, a different saved platform or format version, a changed workspace identity, and filesystem failures return errors. Each command has a separate contained process lifetime.

Termination is idempotent for removed and unknown session identifiers. It leaves existing lock tombstones in place so stale callers remain serialized. Status uses saved state rather than caller-editable metadata.

MXC's template is a host-local JSON policy. Its process sandboxes execute the host's programs using Seatbelt, Bubblewrap, or ProcessContainer. Use the image preparation and OCI layout APIs with image-based providers; MXC reports those operations as `unsupported_operation` and keeps them outside its advertised capabilities.
