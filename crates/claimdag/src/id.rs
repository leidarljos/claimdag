//! Opaque 128-bit work id. Same bit layout as a pair of u64 halves.

use xxhash_rust::xxh3::xxh3_128;

/// Wire id is two u64 halves (xxHash3-128). Zero = unset / mint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkId {
    pub hi: u64,
    pub lo: u64,
}

impl WorkId {
    pub const ZERO: Self = Self { hi: 0, lo: 0 };

    pub fn is_zero(self) -> bool {
        self.hi == 0 && self.lo == 0
    }

    /// 32 lowercase hex chars. Off-disk / CLI only.
    pub fn to_hex(self) -> String {
        format!("{:016x}{:016x}", self.hi, self.lo)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.len() != 32 {
            return None;
        }
        let hi = u64::from_str_radix(&s[..16], 16).ok()?;
        let lo = u64::from_str_radix(&s[16..], 16).ok()?;
        Some(Self { hi, lo })
    }

    pub fn to_be_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.hi.to_be_bytes());
        out[8..].copy_from_slice(&self.lo.to_be_bytes());
        out
    }

    pub fn from_u128(v: u128) -> Self {
        Self {
            hi: (v >> 64) as u64,
            lo: v as u64,
        }
    }

    pub fn to_u128(self) -> u128 {
        ((self.hi as u128) << 64) | (self.lo as u128)
    }
}

fn mint_id_xxh3(parts: &[&[u8]]) -> WorkId {
    let mut buf: Vec<u8> = Vec::with_capacity(parts.iter().map(|p| 4 + p.len()).sum());
    for p in parts {
        let n = p.len() as u32;
        buf.extend_from_slice(&n.to_le_bytes());
        buf.extend_from_slice(p);
    }
    WorkId::from_u128(xxh3_128(&buf))
}

/// Same length-prefixed xxh3 family the seat already minted with.
pub fn mint_work_id(
    parent: WorkId,
    root: WorkId,
    role: &str,
    summary: &str,
    seq: u64,
    salt: &[u8],
) -> WorkId {
    let prompt_hash = xxh3_128(summary.as_bytes()).to_le_bytes();
    mint_id_xxh3(&[
        &parent.to_be_bytes(),
        &root.to_be_bytes(),
        role.as_bytes(),
        b"",
        &prompt_hash,
        &seq.to_le_bytes(),
        salt,
    ])
}
