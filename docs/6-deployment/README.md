# Just-Pkg Deployment Guide

Justpkg is a pure-Rust Nix package resolver and installer. It resolves Nix package attributes to store paths and downloads/installs them into a destination directory.

## Prerequisites by Environment

### Windows
- **Rust 1.70+** (for building from source)
- **Git** (with SSH configured for GitHub access)
- **PowerShell 7+** or **Git Bash**

### Linux / WSL2
- **Rust 1.70+** (for building from source)
- **Git** (with SSH configured for GitHub access)
- **GCC / build-essential** (for linking; pure Rust, minimal C deps)

---

## Installation / Build

### Windows

#### Option A: Build from Source
```powershell
cd C:\phd-systems\swelabs\virtualization\justpkg
cargo build --release

# Binary location
.\target\release\pkg.exe
```

**Add to PATH (optional):**
```powershell
# PowerShell (temporary, current session only)
$env:PATH = "C:\phd-systems\swelabs\virtualization\justpkg\target\release;" + $env:PATH

# Permanent: add to System Environment Variables
# or: [Environment]::SetEnvironmentVariable("PATH", "C:\phd-systems\swelabs\virtualization\justpkg\target\release;" + $env:PATH, "User")
```

#### Option B: Download Prebuilt Release
Check GitHub releases for `pkg-x86_64-pc-windows-msvc` binary (if available).

---

### Linux / WSL2

#### Option A: Build from Source
```bash
cd /path/to/justpkg
cargo build --release

# Binary location
./target/release/pkg

# Add to PATH (temporary, current shell)
export PATH="./target/release:$PATH"

# Add to PATH (permanent, ~/.bashrc or ~/.zshrc)
echo 'export PATH="$HOME/path/to/justpkg/target/release:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

#### Option B: Download Prebuilt Release
```bash
# Check GitHub releases for pkg-x86_64-unknown-linux-gnu binary
# wget https://github.com/sweengineeringlabs/justpkg/releases/download/v0.1.2/pkg-x86_64-unknown-linux-gnu
# chmod +x pkg
# mv pkg ~/.local/bin/
```

---

## Verification

### Windows
```powershell
.\target\release\pkg.exe --version
.\target\release\pkg.exe resolve --help
```

### Linux / WSL2
```bash
./target/release/pkg --version
./target/release/pkg resolve --help
```

---

## Usage Example

### Resolve Nix Packages (Windows or Linux)
```bash
# Windows (PowerShell)
.\target\release\pkg.exe resolve packages.toml --out manifest.json

# Linux / WSL2
./target/release/pkg resolve packages.toml --out manifest.json
```

### Install Packages
```bash
# Windows
.\target\release\pkg.exe install manifest.json C:\output\dir

# Linux / WSL2
./target/release/pkg install manifest.json /tmp/output
```

---

## Cross-Platform Notes

- **Pure Rust, no C deps**: justpkg compiles natively on Windows, Linux, and macOS
- **SSH Git access required**: Ensures SSH key auth works when resolving Git-sourced dependencies
- **Binary naming**: 
  - Windows: `pkg.exe`
  - Linux/macOS: `pkg` (no extension)

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `cargo not found` | Install Rust: https://rustup.rs |
| `git not found` | Install Git for your platform |
| `SSH key not configured` | Run `ssh-add ~/.ssh/id_rsa` (Linux/macOS) or configure Git SSH in Windows |
| `path too long` (Windows) | Enable long paths: `git config --global core.longpaths true` |

---

## Configuring Substituters

By default justpkg fetches from `cache.nixos.org`. To use a private Attic cache — or any
other binary cache — configure substituters in your user-level `application.toml`.

**Location:**
- Windows: `%APPDATA%\justpkg\application.toml`
- Linux/macOS: `~/.config/justpkg/application.toml`

**Example — Attic private cache with nixos.org fallback:**

```toml
[nix]
# Attic is tried first; a 404 or connection error falls through to cache.nixos.org.
# No --substituter flags needed on the command line when this is configured.

[[nix.substituters]]
url   = "http://127.0.0.1:8080/swe-private"
token = "eyJ..."   # ATTIC_CI_TOKEN from packages/attic/.env

[[nix.substituters]]
url = "https://cache.nixos.org"
```

With this in place, `pkg resolve`, `pkg install`, and `pkg rootfs build` all pick
up Attic automatically — no `--substituter` or `--substituter-token` flags needed.

**Substituter fallback:** A 404 (package absent) or transport error (Attic not running)
both advance to the next substituter silently. See `nix/src/api/error.rs: is_cache_miss()`.

**Attic first-time setup:** See `packages/attic/setup.sh` to create the `swe-private`
cache and generate tokens after the Attic VM is running.

---

## Integration with vmisolate

Justpkg is the package resolver and installer in the vmisolate workload packaging pipeline:

```bash
# Pin store paths for a workload
pkg resolve packages/opensearch/packages.toml --out packages/opensearch/manifest.json

# Build the ext4 rootfs image from the [rootfs] section of packages.toml
pkg rootfs build packages/opensearch/packages.toml

# Boot the VM
vmic run --config packages/opensearch/vm.toml
```

See: [vmisolate Workload Packaging Pattern](../../docs/6-deployment/workload_packaging_pattern.md)
