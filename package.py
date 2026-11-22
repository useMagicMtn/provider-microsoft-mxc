"""Build a native MagicMtn provider package (Python 3.11+)."""
import hashlib
import io
import json
import platform
import subprocess
import tarfile
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parent
subprocess.run(["cargo", "build", "--release", "--locked"], cwd=root, check=True)
crate = tomllib.loads((root / "Cargo.toml").read_text())["package"]
name = crate["name"].removeprefix("magicmtn-provider-")
host_os = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}[platform.system()]
arch = {"arm64": "aarch64", "ARM64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64", "AMD64": "x86_64"}[platform.machine()]
exe = root / "target" / "release" / (crate["name"] + (".exe" if host_os == "windows" else ""))
manifest = dict(name=name, version=crate["version"], protocol_version=1, os=host_os,
                arch=arch, sha256=hashlib.sha256(exe.read_bytes()).hexdigest())
(root / "dist").mkdir(exist_ok=True)
output = root / "dist" / f"{name}-{crate['version']}-{host_os}-{arch}.tar.gz"
with tarfile.open(output, "w:gz", format=tarfile.USTAR_FORMAT) as archive:
    data = json.dumps(manifest).encode()
    info = tarfile.TarInfo("package.json")
    info.size = len(data)
    archive.addfile(info, io.BytesIO(data))
    info = archive.gettarinfo(str(exe), arcname="provider")
    info.mode = 0o755
    with exe.open("rb") as binary:
        archive.addfile(info, binary)
    archive.add(root / "LICENSE", arcname="LICENSE")
print(output)
