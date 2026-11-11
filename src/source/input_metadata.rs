//! Read-only facts from the same opened disk input as the source bytes.
//!
//! No path lookup API: virtual filenames must never become filesystem authority.

/// Identity of a supplied Unix file's target, following symlinks. Hard links and
/// symlink aliases share this identity. It is a snapshot, not write authority or
/// a promise that a path still names the same file later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputIdentity {
    /// Device containing the file.
    pub device: u64,
    /// Inode on that device.
    pub inode: u64,
}

/// Immutable metadata for an actual supplied disk input. Absent for stdin,
/// virtual sources, non-regular files, unavailable metadata and non-Unix hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuppliedInputMetadata {
    /// Identity of the opened target, not of a symlink directory entry.
    pub identity: InputIdentity,
    /// Ruby3.4 `File::Stat#executable?` semantics for this process's effective
    /// user and group membership. This is not `access(X_OK)` (ACL/mount checks),
    /// nor merely the presence of any execute bit for a non-root process.
    pub executable: bool,
}

impl SuppliedInputMetadata {
    pub(crate) fn from_file(file: &std::fs::File) -> Option<Self> {
        #[cfg(unix)]
        {
            use rustix::process::{getegid, geteuid, getgid, getgroups};
            use std::os::unix::fs::MetadataExt;

            let stat = file.metadata().ok()?;
            if !stat.is_file() {
                return None;
            }
            let uid = geteuid().as_raw();
            // Owner/root decisions do not require supplementary group lookup.
            let group_owned = if uid == 0 || uid == stat.uid() {
                false
            } else if stat.gid() == getegid().as_raw() || stat.gid() == getgid().as_raw() {
                true
            } else {
                // Failure is unknown, never a fabricated non-executable file.
                getgroups()
                    .ok()?
                    .iter()
                    .any(|gid| gid.as_raw() == stat.gid())
            };
            Some(Self {
                identity: InputIdentity {
                    device: stat.dev(),
                    inode: stat.ino(),
                },
                executable: executable(stat.mode(), stat.uid(), uid, group_owned),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            None
        }
    }
}

#[cfg(test)]
#[path = "input_metadata_tests.rs"]
mod tests;

#[cfg(unix)]
fn executable(mode: u32, owner: u32, effective_uid: u32, group_owned: bool) -> bool {
    if effective_uid == 0 {
        mode & 0o111 != 0
    } else if effective_uid == owner {
        mode & 0o100 != 0
    } else if group_owned {
        mode & 0o010 != 0
    } else {
        mode & 0o001 != 0
    }
}
