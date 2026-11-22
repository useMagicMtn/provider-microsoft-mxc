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

The template contract accepts JSON policy files. Dockerfiles, OCI layouts, and `image:<reference>|<session>` templates are unsupported by these host process backends. A harness using the image convention or requiring a resident background agent needs a different integration. Installing the provider package does not itself configure a compute selection or harness.
