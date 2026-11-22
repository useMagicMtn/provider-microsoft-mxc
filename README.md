<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/logo-dark.svg">
    <img src=".github/assets/logo-light.svg" alt="MagicMtn logo" width="180" height="180">
  </picture>
</p>

# Microsoft MXC provider for MagicMtn

Run host-native commands in isolated workspaces using the process sandbox for your operating system. [Microsoft MXC](https://github.com/microsoft/mxc)

## Prerequisites

Install the provider on the host running your MagicMtn server. MXC is included in the provider executable; no separate MXC executor is needed.

| Server host | Sandbox backend  | Requirements                                                                                                                                                |
| ----------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS       | Seatbelt         | macOS 15+                                                                                                                                                   |
| Linux       | Bubblewrap       | `bwrap` 0.5+ on PATH and working unprivileged user namespaces                                                                                               |
| Windows     | ProcessContainer | A supported Windows 11 build; see [MXC's OS requirements](https://github.com/microsoft/mxc/blob/main/docs/backends/process-container/os-version-support.md) |

Use a client that creates sandboxes from a local JSON policy file and runs host-native commands. Clients that require OCI images or a persistent background sandbox agent need a provider that supports those workloads.

## Installation and configuration

On an Apple Silicon Mac, install from the [MagicMtn Registry](https://control.magicmtn.ai/registry/microsoft-mxc):

```sh
magic provider install microsoft-mxc
```

For Linux or Windows, [build a package](CONTRIBUTING.md#packaging-and-releases) on the host and install the resulting archive with `magic provider install --file PATH_TO_PACKAGE`.

Create a JSON policy file on the server to choose which host paths commands may read and which paths to hide. Start with [the example policy](examples/policy.json), then follow the [policy and template setup](USAGE.md#host-and-policy-configuration). Each sandbox gets a separate writable workspace.

Set `MAGICMTN_MXC_STATE_DIR` in the server environment to choose where workspaces and policies are stored. The default is `$HOME/.local/share/magicmtn/mxc` on Unix and `%LOCALAPPDATA%\MagicMtn\mxc` on Windows. Keep this directory private to the server account and preserve it while sessions are in use.

## Runtime behavior

The provider supports creation, termination, and bounded command execution. Each command runs in a fresh process sandbox with the workspace as its working directory. Workspace files persist across commands and server restarts; background processes do not persist between executions.

Only the workspace is writable. Networking and UI access are disabled, and host credentials are not forwarded. Commands can read the paths allowed by your policy and the backend's baseline system access. Isolation uses the host operating system's process controls rather than a virtual machine.

Commands have time and output limits. Terminating a session removes its workspace and prevents further execution. SSH and SCP are not supported.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development, testing, and packaging.

## License

[Apache License 2.0](LICENSE).
