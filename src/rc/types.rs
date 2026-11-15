//! Serde types for the rclone rc API responses.

use serde::{Deserialize, Deserializer};

/// A JSON number of any shape.
#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
}

impl Number {
    /// Negative and not-a-number values count as zero; fractions are cut.
    fn to_u64(&self) -> u64 {
        match *self {
            Number::Unsigned(v) => v,
            Number::Signed(v) => v.max(0) as u64,
            // `as` saturates, and NaN becomes 0.
            Number::Float(v) => v as u64,
        }
    }
}

/// A count read leniently. rclone computes some of these with floats and without
/// bounds (a percentage past 100 for a file that grew, an ETA that overflowed), and
/// one odd number must not make the whole answer unreadable: a transfer whose
/// progress cannot be read would stay "running" for ever.
fn count<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Option::<Number>::deserialize(d)?.map_or(0, |n| n.to_u64()))
}

/// [`count`] for a value that may be missing or `null`.
fn optional_count<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(Option::<Number>::deserialize(d)?.map(|n| n.to_u64()))
}

/// A percentage, kept within 0–100.
fn percentage<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    Ok(count(d)?.min(100) as u8)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RcListItem {
    pub name: String,
    pub size: i64,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub mod_time: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RcList {
    pub list: Vec<RcListItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RcVersion {
    pub version: String,
    pub decomposed: [u32; 3],
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RcAbout {
    #[serde(deserialize_with = "optional_count")]
    pub total: Option<u64>,
    #[serde(deserialize_with = "optional_count")]
    pub used: Option<u64>,
    #[serde(deserialize_with = "optional_count")]
    pub free: Option<u64>,
    #[serde(deserialize_with = "optional_count")]
    pub trashed: Option<u64>,
    #[serde(deserialize_with = "optional_count")]
    pub other: Option<u64>,
    #[serde(deserialize_with = "optional_count")]
    pub objects: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RcStats {
    #[serde(deserialize_with = "count")]
    pub bytes: u64,
    #[serde(deserialize_with = "count")]
    pub total_bytes: u64,
    pub speed: f64,
    #[serde(deserialize_with = "optional_count")]
    pub eta: Option<u64>,
    #[serde(deserialize_with = "count")]
    pub errors: u64,
    #[serde(deserialize_with = "count")]
    pub transfers: u64,
    #[serde(deserialize_with = "count")]
    pub total_transfers: u64,
    /// `null` while nothing is moving.
    #[serde(deserialize_with = "list")]
    pub transferring: Vec<RcTransferring>,
    pub last_error: Option<String>,
}

/// A list that rclone may send as `null`.
fn list<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RcTransferring {
    pub name: String,
    pub size: i64,
    #[serde(deserialize_with = "count")]
    pub bytes: u64,
    #[serde(deserialize_with = "percentage")]
    pub percentage: u8,
    pub speed: f64,
    #[serde(deserialize_with = "optional_count")]
    pub eta: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RcJobStatus {
    pub id: u64,
    pub finished: bool,
    pub success: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub group: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RcJobId {
    pub jobid: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RcProviders {
    #[serde(default)]
    pub providers: Vec<RcProvider>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RcProvider {
    pub prefix: String,
    pub description: String,
    pub hide: bool,
    pub options: Vec<RcOption>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RcOption {
    pub name: String,
    pub help: String,
    pub default_str: String,
    pub examples: Option<Vec<RcExample>>,
    pub exclusive: bool,
    pub required: bool,
    pub is_password: bool,
    pub advanced: bool,
    pub sensitive: bool,
    #[serde(rename = "Type")]
    pub kind: String,
    pub hide: u8,
    /// Shown only for these values of the `provider` option (`a,b`, or `!a,b` for all
    /// but those); empty for every one.
    pub provider: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RcExample {
    pub value: String,
    pub help: String,
}

/// The answer of `config/create` / `config/update` in non-interactive mode.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RcConfigOut {
    pub state: String,
    pub option: Option<RcOption>,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn odd_numbers_do_not_spoil_the_stats() {
        // A file that grew while it was copied, an ETA that overflowed, a float for a
        // count: each used to make the whole answer a decode error.
        let s: RcStats = serde_json::from_value(json!({
            "bytes": 10, "totalBytes": 20.0, "speed": 1.5, "eta": -9223372036.854776,
            "errors": 0, "transfers": 1, "totalTransfers": 2,
            "transferring": [
                { "name": "a", "size": 4, "bytes": 12, "percentage": 300, "speed": 0, "eta": null },
                { "name": "b", "size": -1, "bytes": -3, "percentage": -5, "eta": 2.9 }
            ]
        }))
        .unwrap();
        assert_eq!((s.bytes, s.total_bytes, s.eta), (10, 20, Some(0)));
        assert_eq!(s.transferring[0].percentage, 100);
        assert_eq!(s.transferring[0].eta, None);
        assert_eq!(
            (
                s.transferring[1].bytes,
                s.transferring[1].percentage,
                s.transferring[1].eta
            ),
            (0, 0, Some(2))
        );
    }

    #[test]
    fn missing_and_null_fields_take_their_defaults() {
        let s: RcStats = serde_json::from_value(json!({ "transferring": null, "eta": null })).unwrap();
        assert_eq!((s.bytes, s.eta, s.transferring.len()), (0, None, 0));
        let a: RcAbout = serde_json::from_value(json!({ "total": 100, "free": -1 })).unwrap();
        assert_eq!((a.total, a.used, a.free), (Some(100), None, Some(0)));
    }
}
