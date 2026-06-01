use std::path::Path;

use edge_domain::HandlerError;
use ext4::{Ext4Error, Filesystem};
use justpkg_vminit::PackageManifest;

pub(crate) const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const SHEBANG: [u8; 2] = [b'#', b'!'];

/// Verify that every package in `manifest` has valid ELF binaries or shell
/// scripts under its `{store_path}/bin/` directory in the given ext4 image.
pub(crate) fn verify_image(
    image_path: &Path,
    manifest: &PackageManifest,
) -> Result<(), HandlerError> {
    let f = std::fs::File::open(image_path).map_err(|e| {
        HandlerError::ExecutionFailed(format!("open image {}: {e}", image_path.display()))
    })?;
    let mut fs = Filesystem::open(f).map_err(|e| {
        HandlerError::ExecutionFailed(format!("open ext4 {}: {e}", image_path.display()))
    })?;

    let mut all_ok = true;
    for (name, store_path) in &manifest.entries {
        match verify_package(&mut fs, name, store_path) {
            Ok(count) => eprintln!("    ok  {name}: {count} ELF binaries"),
            Err(e) => {
                eprintln!("    FAIL {name}: {e}");
                all_ok = false;
            }
        }
    }
    if !all_ok {
        return Err(HandlerError::ExecutionFailed(
            "one or more packages failed ELF verification".into(),
        ));
    }
    eprintln!("    all {} package(s) verified", manifest.entries.len());
    Ok(())
}

/// Check that every regular file under `{store_path}/bin/` in the open ext4
/// image is an ELF binary or a shell script. Returns the count of binaries
/// found, or 0 if `{store_path}/bin/` does not exist (packages with no
/// binaries are valid).
pub fn verify_package<R: std::io::Read + std::io::Seek>(
    fs: &mut Filesystem<R>,
    name: &str,
    store_path: &str,
) -> Result<usize, HandlerError> {
    fs.open_path(store_path).map_err(|e| {
        HandlerError::NotFound(format!("store path {store_path} not found in image: {e}"))
    })?;

    let bin_path = format!("{store_path}/bin");
    let bin_inode_num = match fs.open_path(&bin_path) {
        Ok(n) => n,
        Err(Ext4Error::NotFound { .. }) => return Ok(0),
        Err(e) => return Err(HandlerError::ExecutionFailed(format!("open {bin_path}: {e}"))),
    };
    let bin_inode = fs
        .read_inode(bin_inode_num)
        .map_err(|e| HandlerError::ExecutionFailed(format!("read inode {bin_path}: {e}")))?;
    let entries = fs
        .read_dir(&bin_inode)
        .map_err(|e| HandlerError::ExecutionFailed(format!("read dir {bin_path}: {e}")))?;

    let mut count = 0usize;
    for entry in &entries {
        if entry.is_unused() {
            continue;
        }
        let entry_name = std::str::from_utf8(&entry.name).unwrap_or("<non-utf8>");
        if entry_name == "." || entry_name == ".." {
            continue;
        }
        let file_path = format!("{bin_path}/{entry_name}");
        let file_inode_num = fs
            .open_path(&file_path)
            .map_err(|e| HandlerError::ExecutionFailed(format!("open {file_path}: {e}")))?;
        let file_inode = fs
            .read_inode(file_inode_num)
            .map_err(|e| HandlerError::ExecutionFailed(format!("read inode {file_path}: {e}")))?;
        if !file_inode.is_regular() {
            continue;
        }
        let data = fs
            .read_file(&file_inode)
            .map_err(|e| HandlerError::ExecutionFailed(format!("read file {file_path}: {e}")))?;
        let is_elf = data.len() >= 4 && data[..4] == ELF_MAGIC;
        let is_script = data.len() >= 2 && data[..2] == SHEBANG;
        if !is_elf && !is_script {
            return Err(HandlerError::ExecutionFailed(format!(
                "{name}: {file_path} is not an ELF binary or shell script (first bytes: {:?})",
                &data[..data.len().min(4)]
            )));
        }
        count += 1;
    }
    Ok(count)
}
