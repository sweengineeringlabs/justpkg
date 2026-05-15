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

## Integration with vmisolate

Justpkg is used in the vmisolate workload packaging pipeline to resolve and install Nix packages during rootfs builds:

```bash
# Example from packages/fleet/build-rootfs.sh
pkg install packages/fleet/manifest.json $DEST
```

See: [vmisolate Workload Packaging Pattern](../../docs/6-deployment/workload_packaging_pattern.md)
