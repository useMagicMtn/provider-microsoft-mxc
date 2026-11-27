# Contributing

Build and test the Microsoft MXC provider with the public MagicMtn SDK. Contributions should preserve its persistent workspace sessions, filesystem policy, and bounded execution contracts.

## Development setup

Use a Rust toolchain supporting edition 2024 and Python 3.11+ for packaging. Python is not a runtime dependency. Native execution requires the host prerequisites in [README.md](README.md); unit and protocol tests do not launch a native sandbox.

Local development uses the SDK and provider framework from a sibling `magicmtn` checkout:

```text
src/
  magicmtn/
  provider-microsoft-mxc/
```

Check out the public SDK revision recorded in [SDK_REVISION](SDK_REVISION). Copy the example patches into an ignored local Cargo configuration:

```sh
cp .cargo/config.toml.example .cargo/config.toml
```

Adjust the paths for your checkout layout, then build:

```sh
cargo build --locked
```

The manifest declares versioned SDK dependencies, while the current lockfile resolves them through local patches. Use compatible SDK sources for locked development builds. A standalone release build needs a lockfile resolved against its chosen dependency source. Preserve the `mxc-sdk = "=1.0.0"` pin unless intentionally updating the containment engine.

## Architecture

- `src/main.rs` runs the persistent JSON RPC service and advertises creation, status, termination, exec, and bounded exec.
- `src/lib.rs` implements `SandboxBackend`, native MXC launch, policy validation, durable workspaces, session locking, and output capture. Its unit tests cover argument quoting, output bounds, invalid templates and session IDs, durable status, and damaged state.
- `tests/protocol.rs` checks discovery, capabilities, state-free startup, saved session status, removal, invalid creation, unsupported SSH, and shutdown through the executable.
- `tests/containment.rs` contains opt-in tests that launch real native sandboxes.
- `examples/policy.json` provides an empty host-access policy.
- `package.py` builds and packages a native executable; `package.sh` invokes it on Unix.

The default `plugin` feature builds the RPC executable. For library embedding, disable default features and construct `MxcProvider::new(private_state_directory)`. The provider selects Seatbelt on macOS, Bubblewrap on Linux, and ProcessContainer on Windows. Other MXC backends are not exposed, and a failed contained launch never falls back to an uncontained process.

The executable constructs `MxcProvider::deferred_from_env()`. Discovery and shutdown perform no environment or filesystem setup. The first workspace operation initializes and pins the state directory using `MAGICMTN_MXC_STATE_DIR` or the platform default. Explicit library construction with `new` or `from_env` retains immediate initialization. Creation checks backend availability, and each execution asks MXC to enforce the policy.

## Lifecycle and execution

State contains `sessions/<UUID>/policy.json`, `sessions/<UUID>/workspace/`, and `locks/<UUID>.lock`. Session IDs have the form `mxc-<canonical UUID>`. Saved policy includes a format version and host platform, which execution checks before launch. Preserve the state directory across runner restarts and upgrades while sessions remain active.

Each execution starts a fresh contained process tree using the saved policy and workspace as its working directory. Files persist between executions; process state and background services do not. Status validates the durable session under its existing lock. It reports `running` for a valid available workspace and `missing` for a removed or unknown session, and preserves errors for damaged state or failed filesystem access. Interactive SSH and SCP are not advertised. The SSH and SCP trait methods remain required unsupported-operation implementations.

Execution and termination take the same exclusive per-session file lock across provider processes. Termination waits for an active execution, removes the session directory, and revokes future execution. Repeating termination of a previously created session succeeds. Lock files remain as small tombstones so stale callers coordinate through the same inode; do not remove them while callers may still hold them.

The child receives an explicit environment. Home and temporary-directory variables point to the workspace. Unix PATH is `/usr/bin:/bin:/usr/sbin:/sbin`; Windows PATH points to `SystemRoot\System32`, with the required Windows directory variables supplied. Other server environment variables and credentials are not forwarded. Child stdin is closed so workloads cannot consume the provider's RPC input.

Arguments are quoted separately to preserve whitespace, quotes, and shell characters. Use an explicit shell such as `/bin/sh -c` or `cmd.exe /d /c` when shell interpretation is intended. Programs must be nonempty, at most 256 bytes, and cannot begin with `-` or contain NUL. There may be at most 32 arguments, each at most 2048 bytes without NUL.

`exec_bounded` accepts 1–30000 ms and 1–16384 output bytes **per stream**. The provider drains stdout and stderr concurrently, bounds retained output including UTF-8 replacement, preserves nonzero exit codes, and reports timeout and truncation. `exec` uses 30 seconds and 16 KiB per stream and errors on timeout or nonzero exit. Process cleanup releases remaining contained descendants and pipe handles before joining output readers. Errors are not marked retryable; never automatically replay a command that may have taken effect.

Network egress, ingress, and host-loopback access are denied. UI access is disabled. Changes to those policies, backend selection, or the child environment require explicit compatibility review and native containment validation.

## Verification

Run the standard checks:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --no-default-features
```

These checks compile and exercise the library and RPC contracts without launching a native sandbox. For containment, lifecycle, or execution changes, also run the opt-in suite on a prepared host:

```sh
cargo test --locked --test containment -- --ignored
```

Native tests cover workspace persistence across provider instances, teardown, and revocation. Unix tests also exercise filesystem and network denial, read-only and denied paths, argument quoting, output pressure, timeouts, and cleanup of background processes holding pipes. Report native results by operating system; a unit-test pass does not establish native containment support.

The existing GitHub Actions workflow runs formatting, lint, and unit/protocol checks on macOS, Linux, and Windows, plus native containment tests on macOS and Linux. Windows native tests require a supported Windows 11 host. CI checks out the public `useMagicMtn/magicmtn` repository at [SDK_REVISION](SDK_REVISION) and copies the example Cargo patches. `MAGICMTN_SDK_REPOSITORY` and `MAGICMTN_SDK_REF` repository variables can select a compatible test revision. Its local patch paths match the sibling checkout layout.

## Packaging and releases

After configuring the SDK patches, build on the operating system and architecture you intend to distribute:

```sh
python3 package.py
```

On Windows, use `python package.py`. On Unix, `./package.sh` is equivalent. Packaging requires Python 3.11+ for `tomllib` and runs `cargo build --release --locked`.

The output is `dist/microsoft-mxc-<version>-<os>-<arch>.tar.gz`, containing `provider`, `package.json`, and `LICENSE`. The packager handles the Windows `.exe` suffix and records native platform metadata, protocol version 1, and the executable's SHA-256 checksum. MXC is compiled into the executable; Linux Bubblewrap remains a host prerequisite.

Install the matching archive on the MagicMtn server host. For example, on an Apple Silicon Mac:

```sh
magic provider install --file dist/microsoft-mxc-0.2.0-macos-aarch64.tar.gz
```

Before publishing a release, verify discovery, archive contents, checksum, and a native create/execute/terminate cycle on each advertised platform. Use new versions and digests for changed released binaries and retain previous releases for existing session bindings. Preserve the Apache-2.0 license and keep local configuration, private templates, and session state out of distributions.

## Upstream references

- [MXC Rust API](https://github.com/microsoft/mxc/tree/main/docs/api-reference/rust/v1)
- [Seatbelt backend](https://github.com/microsoft/mxc/blob/main/docs/backends/seatbelt/seatbelt-backend.md)
- [Bubblewrap backend](https://github.com/microsoft/mxc/blob/main/docs/backends/bwrap/bubblewrap-backend.md)
- [Container lifecycle](https://github.com/microsoft/mxc/blob/main/docs/container-lifecycle.md)
