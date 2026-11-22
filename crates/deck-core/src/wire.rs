//! Serde helpers that carry paths and environment strings losslessly as bytes (Unix only),
//! for `#[serde(with = "...")]`. serde's own `Path` impl fails on non-UTF-8 names.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn bytes(s: &OsStr) -> &[u8] {
    s.as_bytes()
}

pub mod path {
    use super::*;

    pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
        bytes(p.as_os_str()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        Ok(PathBuf::from(OsString::from_vec(Vec::<u8>::deserialize(
            d,
        )?)))
    }
}

pub mod opt_path {
    use super::*;

    pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
        p.as_ref().map(|p| bytes(p.as_os_str())).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
        Ok(Option::<Vec<u8>>::deserialize(d)?.map(|v| PathBuf::from(OsString::from_vec(v))))
    }
}

pub mod env {
    use super::*;

    pub fn serialize<S: Serializer>(v: &[(OsString, OsString)], s: S) -> Result<S::Ok, S::Error> {
        let pairs: Vec<(&[u8], &[u8])> = v.iter().map(|(k, v)| (bytes(k), bytes(v))).collect();
        pairs.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<(OsString, OsString)>, D::Error> {
        let pairs = Vec::<(Vec<u8>, Vec<u8>)>::deserialize(d)?;
        Ok(pairs
            .into_iter()
            .map(|(k, v)| (OsString::from_vec(k), OsString::from_vec(v)))
            .collect())
    }
}
