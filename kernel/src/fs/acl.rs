//! POSIX & Win32 Access Control List (ACL) and Security Identifier (SID) Models.
//!
//! Provides granular file permission enforcement, Access Control Entries (ACE),
//! POSIX mode bits (rwx for owner/group/others), and Win32 discretionary access control (DACL).

use alloc::string::String;
use alloc::vec::Vec;

/// Standard file access rights bitmask combining POSIX and NT file access masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileAccessMask {
    pub bits: u32,
}

impl FileAccessMask {
    pub const READ: u32 = 0x0001;
    pub const WRITE: u32 = 0x0002;
    pub const EXECUTE: u32 = 0x0004;
    pub const DELETE: u32 = 0x0008;
    pub const READ_ATTRIBUTES: u32 = 0x0010;
    pub const WRITE_ATTRIBUTES: u32 = 0x0020;
    pub const SYNCHRONIZE: u32 = 0x0040;
    pub const APPEND: u32 = 0x0080;

    // Generic composite masks
    pub const GENERIC_READ: u32 = Self::READ | Self::READ_ATTRIBUTES;
    pub const GENERIC_WRITE: u32 = Self::WRITE | Self::WRITE_ATTRIBUTES | Self::APPEND;
    pub const GENERIC_EXECUTE: u32 = Self::EXECUTE | Self::READ_ATTRIBUTES;
    pub const GENERIC_ALL: u32 = 0x00FF;

    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    pub const fn read() -> Self {
        Self { bits: Self::READ }
    }

    pub const fn write() -> Self {
        Self { bits: Self::WRITE }
    }

    pub const fn execute() -> Self {
        Self { bits: Self::EXECUTE }
    }

    pub const fn read_write() -> Self {
        Self { bits: Self::READ | Self::WRITE }
    }

    pub const fn all() -> Self {
        Self { bits: Self::GENERIC_ALL }
    }

    pub fn contains(&self, other: FileAccessMask) -> bool {
        (self.bits & other.bits) == other.bits
    }

    pub fn intersects(&self, other: FileAccessMask) -> bool {
        (self.bits & other.bits) != 0
    }

    pub fn union(&self, other: FileAccessMask) -> Self {
        Self { bits: self.bits | other.bits }
    }
}

/// Security Identifier representing an individual user, group, or system entity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SecurityIdentifier {
    pub uid: u32,
    pub gid: u32,
    pub sid_str: String,
}

impl SecurityIdentifier {
    pub fn new(uid: u32, gid: u32, sid_str: &str) -> Self {
        Self {
            uid,
            gid,
            sid_str: String::from(sid_str),
        }
    }

    /// Root/Superuser SID (SYSTEM: S-1-5-18).
    pub fn root() -> Self {
        Self::new(0, 0, "S-1-5-18")
    }

    /// Administrator SID (Administrators: S-1-5-32-544).
    pub fn admin() -> Self {
        Self::new(1000, 1000, "S-1-5-32-544")
    }

    /// Standard user SID (Users: S-1-5-32-545).
    pub fn user(uid: u32, gid: u32) -> Self {
        Self::new(uid, gid, "S-1-5-32-545")
    }

    /// World/Everyone SID (Everyone: S-1-1-0).
    pub fn everyone() -> Self {
        Self::new(65534, 65534, "S-1-1-0")
    }

    pub fn is_root(&self) -> bool {
        self.uid == 0 || self.sid_str == "S-1-5-18"
    }
}

/// Individual POSIX read, write, execute permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModePermissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl ModePermissions {
    pub const fn new(read: bool, write: bool, execute: bool) -> Self {
        Self { read, write, execute }
    }

    pub const fn rwx() -> Self {
        Self { read: true, write: true, execute: true }
    }

    pub const fn rx() -> Self {
        Self { read: true, write: false, execute: true }
    }

    pub const fn ro() -> Self {
        Self { read: true, write: false, execute: false }
    }

    pub const fn none() -> Self {
        Self { read: false, write: false, execute: false }
    }

    pub fn to_bits(&self) -> u32 {
        let mut bits = 0;
        if self.read { bits |= 4; }
        if self.write { bits |= 2; }
        if self.execute { bits |= 1; }
        bits
    }

    pub fn from_bits(bits: u32) -> Self {
        Self {
            read: (bits & 4) != 0,
            write: (bits & 2) != 0,
            execute: (bits & 1) != 0,
        }
    }

    pub fn allows(&self, mask: FileAccessMask) -> bool {
        if (mask.bits & FileAccessMask::READ) != 0 && !self.read {
            return false;
        }
        if (mask.bits & (FileAccessMask::WRITE | FileAccessMask::DELETE | FileAccessMask::APPEND)) != 0 && !self.write {
            return false;
        }
        if (mask.bits & FileAccessMask::EXECUTE) != 0 && !self.execute {
            return false;
        }
        true
    }
}

/// POSIX Mode Permissions covering owner, group, others, and special mode bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePermissions {
    pub owner: ModePermissions,
    pub group: ModePermissions,
    pub others: ModePermissions,
    pub setuid: bool,
    pub setgid: bool,
    pub sticky: bool,
}

impl FilePermissions {
    pub const fn new(owner: ModePermissions, group: ModePermissions, others: ModePermissions) -> Self {
        Self {
            owner,
            group,
            others,
            setuid: false,
            setgid: false,
            sticky: false,
        }
    }

    pub const fn default_file() -> Self {
        Self::new(
            ModePermissions::new(true, true, false), // 6: rw-
            ModePermissions::new(true, false, false), // 4: r--
            ModePermissions::new(true, false, false), // 4: r--
        )
    }

    pub const fn default_dir() -> Self {
        Self::new(
            ModePermissions::new(true, true, true), // 7: rwx
            ModePermissions::new(true, false, true), // 5: r-x
            ModePermissions::new(true, false, true), // 5: r-x
        )
    }

    pub const fn all_rwx() -> Self {
        Self::new(
            ModePermissions::rwx(),
            ModePermissions::rwx(),
            ModePermissions::rwx(),
        )
    }

    pub fn from_octal(mode: u32) -> Self {
        let setuid = (mode & 0o4000) != 0;
        let setgid = (mode & 0o2000) != 0;
        let sticky = (mode & 0o1000) != 0;
        let owner = ModePermissions::from_bits((mode >> 6) & 0o7);
        let group = ModePermissions::from_bits((mode >> 3) & 0o7);
        let others = ModePermissions::from_bits(mode & 0o7);
        Self {
            owner,
            group,
            others,
            setuid,
            setgid,
            sticky,
        }
    }

    pub fn to_octal(&self) -> u32 {
        let mut mode = 0;
        if self.setuid { mode |= 0o4000; }
        if self.setgid { mode |= 0o2000; }
        if self.sticky { mode |= 0o1000; }
        mode |= self.owner.to_bits() << 6;
        mode |= self.group.to_bits() << 3;
        mode |= self.others.to_bits();
        mode
    }

    /// Evaluates POSIX permission for a subject against this file's permissions.
    pub fn check_permission(&self, owner_sid: &SecurityIdentifier, subject_sid: &SecurityIdentifier, mask: FileAccessMask) -> bool {
        if subject_sid.is_root() {
            return true;
        }

        if subject_sid.uid == owner_sid.uid {
            return self.owner.allows(mask);
        }

        if subject_sid.gid == owner_sid.gid {
            return self.group.allows(mask);
        }

        self.others.allows(mask)
    }
}

/// Access Control Entry (ACE) Type: Allow or Deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AceType {
    AccessAllowed,
    AccessDenied,
}

/// Discretionary Access Control Entry (ACE) defining permissions for a specific SID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessControlEntry {
    pub ace_type: AceType,
    pub sid: SecurityIdentifier,
    pub mask: FileAccessMask,
    pub flags: u8,
}

impl AccessControlEntry {
    pub const INHERIT_OBJECT: u8 = 0x01;
    pub const INHERIT_CONTAINER: u8 = 0x02;

    pub fn allow(sid: SecurityIdentifier, mask: FileAccessMask) -> Self {
        Self {
            ace_type: AceType::AccessAllowed,
            sid,
            mask,
            flags: 0,
        }
    }

    pub fn deny(sid: SecurityIdentifier, mask: FileAccessMask) -> Self {
        Self {
            ace_type: AceType::AccessDenied,
            sid,
            mask,
            flags: 0,
        }
    }
}

/// Discretionary Access Control List (DACL) managing explicit Allow/Deny entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessControlList {
    pub entries: Vec<AccessControlEntry>,
}

impl AccessControlList {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add_entry(&mut self, entry: AccessControlEntry) {
        self.entries.push(entry);
    }

    pub fn allow(&mut self, sid: SecurityIdentifier, mask: FileAccessMask) {
        self.add_entry(AccessControlEntry::allow(sid, mask));
    }

    pub fn deny(&mut self, sid: SecurityIdentifier, mask: FileAccessMask) {
        self.add_entry(AccessControlEntry::deny(sid, mask));
    }

    /// Evaluates requested access mask against ACL entries.
    /// Follows standard NT DACL evaluation:
    /// 1. Deny ACE matching subject SID immediately denies the intersecting bits.
    /// 2. Allow ACE matching subject SID or Everyone grants rights.
    /// 3. Returns true only if all requested bits are granted and none denied.
    pub fn check_access(&self, subject: &SecurityIdentifier, requested: FileAccessMask) -> bool {
        if subject.is_root() {
            return true;
        }

        let mut remaining_needed = requested.bits;

        // Process explicit Deny ACEs first
        for ace in &self.entries {
            if ace.ace_type == AceType::AccessDenied {
                if ace.sid == *subject || ace.sid == SecurityIdentifier::everyone() {
                    if (ace.mask.bits & requested.bits) != 0 {
                        return false;
                    }
                }
            }
        }

        // Process Allow ACEs
        for ace in &self.entries {
            if ace.ace_type == AceType::AccessAllowed {
                if ace.sid == *subject || ace.sid == SecurityIdentifier::everyone() {
                    remaining_needed &= !ace.mask.bits;
                    if remaining_needed == 0 {
                        return true;
                    }
                }
            }
        }

        remaining_needed == 0
    }
}
