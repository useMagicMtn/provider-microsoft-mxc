# Agent development guide

## Coding style

Use the [Google Rust Guide](https://google.github.io/styleguide/rustguide.html). Keep packaging code consistent with the existing Python standard-library script.

## Setup and commands

See [CONTRIBUTING.md](CONTRIBUTING.md) for SDK patches, runtime contracts, testing, and releases. See the [usage guide](USAGE.md) for host and policy configuration. Locked local builds require compatible SDK sources through the ignored `.cargo/config.toml`.

- Build: `cargo build --locked`
- Unit, protocol, and doc tests: `cargo test --locked`
- Library without the plugin: `cargo test --locked --no-default-features`
- Formatting: `cargo fmt --check` (apply with `cargo fmt`)
- Lint: `cargo clippy --locked --all-targets -- -D warnings`
- Package: `python3 package.py` (Windows: `python package.py`)
- Native containment: `cargo test --locked --test containment -- --ignored`

Run checks relevant to the change. Runtime changes need default-feature and library-only checks. Native tests launch real sandboxes and require a prepared host; report their results separately by platform. Do not change host security settings or install containment backends as part of routine checks.

## Repository structure

- `src/main.rs`: RPC entry point and capability advertisement.
- `src/lib.rs`: SDK implementation, policy and state handling, locks, native containment, bounded output, and unit tests.
- `tests/protocol.rs`: persistent executable protocol tests.
- `tests/containment.rs`: opt-in native containment tests.
- `examples/policy.json`: minimal policy template.
- `package.py` and `package.sh`: native release packaging.
- `README.md`: user overview, prerequisites, installation, and behavior.
- `CONTRIBUTING.md`: development and operational details.

## Compatibility and runtime changes

- Preserve create, terminate, exec, and bounded exec capabilities. Do not advertise status, SSH, or SCP without implementing their contracts. Required unsupported trait methods are not dead code.
- Keep the selected Seatbelt, Bubblewrap, and ProcessContainer backends. Never fall back to an uncontained process.
- Preserve strict template validation, canonical session IDs, saved policy version/platform checks, and rejection of paths overlapping provider state. Do not silently add OCI or Dockerfile support.
- Preserve private workspaces, the explicit child environment, closed stdin, network/UI denial, and separately quoted arguments. Review containment changes against native tests on affected platforms.
- Preserve per-session cross-process locking and lock tombstones. Termination must serialize with exec and revoke future execution without breaking stale callers' locks.
- Preserve per-stream byte bounds, concurrent draining, timeouts, process-tree cleanup, and nonzero exit reporting. Do not automatically retry guest commands or mark uncertain execution errors retryable.
- Keep `Cargo.lock` and the pinned MXC SDK stable unless intentionally changing dependencies. Do not commit `.cargo/config.toml`, `target/`, `dist/`, private policies, credentials, or session state.
- Preserve the Apache-2.0 license, previous releases, and active session bindings. Released binary changes require new versions and digests.
- Keep the README focused on users; put build, API, testing, and release details in CONTRIBUTING.md. Preserve the existing CI workflow's platform and feature coverage.
