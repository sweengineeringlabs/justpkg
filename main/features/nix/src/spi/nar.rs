//! NAR (Nix ARchive) extractor.
//!
//! NAR is a deterministic archive format. Wire format: length-prefixed
//! UTF-8 strings (8-byte LE length, padded to 8-byte boundary), with
//! a recursive tree of file/directory/symlink nodes.
//!
//! Every path is passed through `safe_path_join` before writing.

use std::io::Read;
use std::path::Path;

use justpkg_pkg::safe_path_join;
use crate::api::error::NixFetchError;

const NAR_MAGIC: &str = "nix-archive-1";

pub fn extract_nar<R: Read>(mut reader: R, dest: &Path) -> Result<(), NixFetchError> {
    let magic = read_nar_str(&mut reader)?;
    if magic != NAR_MAGIC {
        return Err(NixFetchError::NarExtract(
            format!("invalid NAR magic: expected '{NAR_MAGIC}', got '{magic}'")
        ));
    }
    read_nar_node(&mut reader, dest, dest)
}

fn read_nar_str<R: Read>(r: &mut R) -> Result<String, NixFetchError> {
    let mut len_buf = [0u8; 8];
    r.read_exact(&mut len_buf)
        .map_err(|e| NixFetchError::NarExtract(format!("read length: {e}")))?;
    let len = u64::from_le_bytes(len_buf) as usize;

    let padded = (len + 7) & !7;
    let mut buf = vec![0u8; padded];
    r.read_exact(&mut buf)
        .map_err(|e| NixFetchError::NarExtract(format!("read string body: {e}")))?;

    String::from_utf8(buf[..len].to_vec())
        .map_err(|e| NixFetchError::NarExtract(format!("invalid UTF-8: {e}")))
}

fn read_nar_node<R: Read>(r: &mut R, current: &Path, base: &Path) -> Result<(), NixFetchError> {
    expect_str(r, "(")?;
    expect_str(r, "type")?;
    let node_type = read_nar_str(r)?;

    match node_type.as_str() {
        "regular" => read_regular(r, current)?,
        "directory" => read_directory(r, current, base)?,
        "symlink" => read_symlink(r, current)?,
        other => return Err(NixFetchError::NarExtract(
            format!("unknown NAR node type: {other}")
        )),
    }

    expect_str(r, ")")?;
    Ok(())
}

fn read_regular<R: Read>(r: &mut R, path: &Path) -> Result<(), NixFetchError> {
    #[cfg(unix)]
    let mut executable = false;
    loop {
        let field = read_nar_str(r)?;
        match field.as_str() {
            "executable" => {
                expect_str(r, "")?;
                #[cfg(unix)]
                { executable = true; }
            }
            "contents" => {
                let mut len_buf = [0u8; 8];
                r.read_exact(&mut len_buf)
                    .map_err(|e| NixFetchError::NarExtract(format!("read contents len: {e}")))?;
                let len = u64::from_le_bytes(len_buf) as usize;
                let padded = (len + 7) & !7;

                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| NixFetchError::NarExtract(format!("mkdir {parent:?}: {e}")))?;
                }

                let mut file = std::fs::File::create(path)
                    .map_err(|e| NixFetchError::NarExtract(format!("create {path:?}: {e}")))?;

                let mut buf = vec![0u8; padded];
                r.read_exact(&mut buf)
                    .map_err(|e| NixFetchError::NarExtract(format!("read contents: {e}")))?;

                use std::io::Write;
                file.write_all(&buf[..len])
                    .map_err(|e| NixFetchError::NarExtract(format!("write {path:?}: {e}")))?;

                #[cfg(unix)]
                if executable {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| NixFetchError::NarExtract(format!("chmod {path:?}: {e}")))?;
                }
            }
            ")" => return Ok(()),
            other => return Err(NixFetchError::NarExtract(
                format!("unexpected field in regular node: {other}")
            )),
        }
    }
}

fn read_directory<R: Read>(r: &mut R, path: &Path, base: &Path) -> Result<(), NixFetchError> {
    std::fs::create_dir_all(path)
        .map_err(|e| NixFetchError::NarExtract(format!("mkdir {path:?}: {e}")))?;

    loop {
        let field = read_nar_str(r)?;
        match field.as_str() {
            "entry" => {
                expect_str(r, "(")?;
                expect_str(r, "name")?;
                let name = read_nar_str(r)?;
                let child = safe_path_join(path, &name)
                    .map_err(|e| NixFetchError::NarExtract(format!("unsafe path '{name}': {e}")))?;
                expect_str(r, "node")?;
                read_nar_node(r, &child, base)?;
                expect_str(r, ")")?;
            }
            ")" => return Ok(()),
            other => return Err(NixFetchError::NarExtract(
                format!("unexpected field in directory node: {other}")
            )),
        }
    }
}

fn read_symlink<R: Read>(r: &mut R, path: &Path) -> Result<(), NixFetchError> {
    expect_str(r, "target")?;
    let _target = read_nar_str(r)?;

    #[cfg(unix)]
    std::os::unix::fs::symlink(&_target, path)
        .map_err(|e| NixFetchError::NarExtract(format!("symlink {path:?} -> {_target}: {e}")))?;

    // On Windows: write target as a text file (symlinks require elevated privileges)
    #[cfg(windows)]
    {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| NixFetchError::NarExtract(format!("mkdir: {e}")))?;
        }
        let mut f = std::fs::File::create(path)
            .map_err(|e| NixFetchError::NarExtract(format!("create symlink stub: {e}")))?;
        writeln!(f, "{_target}")
            .map_err(|e| NixFetchError::NarExtract(format!("write symlink stub: {e}")))?;
    }

    Ok(())
}

fn expect_str<R: Read>(r: &mut R, expected: &str) -> Result<(), NixFetchError> {
    let got = read_nar_str(r)?;
    if got != expected {
        return Err(NixFetchError::NarExtract(
            format!("expected '{expected}', got '{got}'")
        ));
    }
    Ok(())
}
