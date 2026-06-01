use ext4::Filesystem;
use swe_justpkg_rootfs::build_rootfs;
use std::io::Write as _;

// ── Happy path ────────────────────────────────────────────────────────────────

#[test]
fn test_build_rootfs_binary_file_dir_entries_land_in_ext4_image() {
    // Uses an empty manifest ({"packages": {}}) so install_packages() short-circuits
    // with no network access. Verifies [[rootfs.binary]], [[rootfs.file]], and
    // [[rootfs.dir]] all land in the ext4 image with the correct content.

    let fixture = tempfile::tempdir().unwrap();
    let pkg_dir = fixture.path();

    std::fs::write(pkg_dir.join("packages.toml"), r##"
nixpkgs_channel = "nixos-24.11"
system          = "x86_64-linux"

[[package]]
attr = "bash"
name = "bash"

[rootfs]
image_out = "dist/test.ext4"

[[rootfs.dir]]
path = "/var/data"

[[rootfs.file]]
path    = "/etc/greeting"
content = "hello from rootfs test\n"
mode    = 420

[[rootfs.binary]]
path   = "/usr/local/bin/myapp"
source = "myapp"
"##).unwrap();

    std::fs::write(pkg_dir.join("manifest.json"), r#"{"packages": {}}"#).unwrap();

    let mut f = std::fs::File::create(pkg_dir.join("myapp")).unwrap();
    f.write_all(b"\x7fELFfake-content-for-test").unwrap();
    drop(f);

    let image_path = pkg_dir.join("test.ext4");
    let result = build_rootfs(
        &pkg_dir.join("packages.toml"),
        Some(image_path.clone()),
        vec![],
        &justpkg_config::AppConfig::default(),
    );
    assert!(result.is_ok(), "build_rootfs must succeed: {:?}", result.err());
    assert!(image_path.exists(), "ext4 image must be created");

    let mut fs = Filesystem::open(std::fs::File::open(&image_path).unwrap())
        .expect("must open ext4 image");

    // [[rootfs.binary]]: ELF magic and exact content preserved
    let binary = read_from_image(&mut fs, "/usr/local/bin/myapp");
    assert!(
        binary.starts_with(b"\x7fELF"),
        "injected binary must start with ELF magic; got {:?}",
        &binary[..binary.len().min(8)]
    );
    assert_eq!(&binary[4..], b"fake-content-for-test", "binary content must be preserved verbatim");

    // [[rootfs.file]]: text content exact (no placeholders in this fixture)
    let greeting = read_from_image(&mut fs, "/etc/greeting");
    assert_eq!(greeting, b"hello from rootfs test\n");

    // [[rootfs.dir]]: directory present
    assert!(fs.open_path("/var/data").is_ok(), "/var/data must exist in image");
}

// ── Sad paths ─────────────────────────────────────────────────────────────────

#[test]
fn test_build_rootfs_absent_manifest_returns_error() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("packages.toml"), r#"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
"#).unwrap();

    let result = build_rootfs(
        &fixture.path().join("packages.toml"),
        Some(fixture.path().join("out.ext4")),
        vec![],
        &justpkg_config::AppConfig::default(),
    );
    assert!(result.is_err(), "must fail when manifest.json is absent");
    assert!(
        matches!(result.unwrap_err(), edge_domain::HandlerError::InvalidRequest(_)),
        "error kind must be InvalidRequest"
    );
}

#[test]
fn test_build_rootfs_missing_binary_source_returns_error() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("packages.toml"), r#"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
[[rootfs.binary]]
path   = "/usr/local/bin/ghost"
source = "ghost"
"#).unwrap();
    std::fs::write(fixture.path().join("manifest.json"), r#"{"packages": {}}"#).unwrap();

    let result = build_rootfs(
        &fixture.path().join("packages.toml"),
        Some(fixture.path().join("out.ext4")),
        vec![],
        &justpkg_config::AppConfig::default(),
    );
    assert!(result.is_err(), "must fail when [[rootfs.binary]] source is absent");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_from_image<R: std::io::Read + std::io::Seek>(
    fs: &mut Filesystem<R>,
    vfs_path: &str,
) -> Vec<u8> {
    let inode_num = fs
        .open_path(vfs_path)
        .unwrap_or_else(|e| panic!("open_path {vfs_path:?}: {e}"));
    let inode = fs
        .read_inode(inode_num)
        .unwrap_or_else(|e| panic!("read_inode {vfs_path:?}: {e}"));
    fs.read_file(&inode)
        .unwrap_or_else(|e| panic!("read_file {vfs_path:?}: {e}"))
}
