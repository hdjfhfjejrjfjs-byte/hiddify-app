//! Built-in route rules generation, legacy key migration, region-aware merging,
//! duplication, and contiguous `list_order` renumbering (`rules_notifier.dart`).
//!
//! Operates directly on `package:protobuf`'s `RouteRule.writeToJson()` tag-numbered JSON format
//! so all 18 `Rule` fields (including custom user fields) round-trip losslessly.

use serde_json::{json, Map, Value};

use crate::ConfigError;

const GEO_URL: &str = "https://raw.githubusercontent.com/hiddify/hiddify-geo/rule-set";
const GEOSITE_URL: &str = "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set";

// Protobuf `Outbound` enum values (`route_rule.pbenum.dart`):
// 0 = proxy, 1 = direct, 2 = direct_with_fragment, 3 = block
const OUTBOUND_DIRECT: i64 = 1;
const OUTBOUND_BLOCK: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinRuleKind {
    BypassLan,
    SystemDirect,
    BlockAds,
    BypassRegion,
}

impl BuiltinRuleKind {
    pub const ALL: [Self; 4] = [
        Self::BypassLan,
        Self::SystemDirect,
        Self::BlockAds,
        Self::BypassRegion,
    ];

    pub fn short_name(self) -> &'static str {
        match self {
            Self::BypassLan => "bypassLan",
            Self::SystemDirect => "systemDirect",
            Self::BlockAds => "blockAds",
            Self::BypassRegion => "bypassRegion",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::BypassLan => "builtin:bypassLan",
            Self::SystemDirect => "builtin:systemDirect",
            Self::BlockAds => "builtin:blockAds",
            Self::BypassRegion => "builtin:bypassRegion",
        }
    }

    pub fn enabled_by_default(self) -> bool {
        match self {
            Self::BypassLan | Self::BypassRegion => true,
            Self::SystemDirect | Self::BlockAds => false,
        }
    }

    pub fn from_rule_name(name: &str) -> Option<Self> {
        match name {
            "builtin:bypassLan" => Some(Self::BypassLan),
            "builtin:systemDirect" | "builtin:appleDirect" => Some(Self::SystemDirect),
            "builtin:blockAds" => Some(Self::BlockAds),
            "builtin:bypassRegion" => Some(Self::BypassRegion),
            _ => None,
        }
    }

    pub fn from_short_name(short: &str) -> Option<Self> {
        match short {
            "bypassLan" => Some(Self::BypassLan),
            "systemDirect" => Some(Self::SystemDirect),
            "blockAds" => Some(Self::BlockAds),
            "bypassRegion" => Some(Self::BypassRegion),
            _ => None,
        }
    }

    /// Builds a `Rule` map in `package:protobuf` tag-numbered JSON format (`Rule.writeToJsonMap()`).
    pub fn build_pb_map(self, platform: &str) -> Map<String, Value> {
        let mut rule = Map::new();
        match self {
            Self::BypassLan => {
                rule.insert("3".to_string(), json!(self.key()));
                rule.insert("4".to_string(), json!(OUTBOUND_DIRECT));
                rule.insert(
                    "13".to_string(),
                    json!([
                        "10.0.0.0/8",
                        "172.16.0.0/12",
                        "192.168.0.0/16",
                        "127.0.0.0/8",
                        "169.254.0.0/16",
                        "224.0.0.0/4",
                        "0.0.0.0/32",
                        "fc00::/7",
                        "fe80::/10",
                        "::1/128",
                        "ff00::/8",
                        "::/128",
                    ]),
                );
            }
            Self::SystemDirect => {
                rule.insert("3".to_string(), json!(self.key()));
                rule.insert("4".to_string(), json!(OUTBOUND_DIRECT));
                match platform.to_ascii_lowercase().as_str() {
                    "ios" | "macos" | "apple" => {
                        rule.insert(
                            "5".to_string(),
                            json!([format!("{GEOSITE_URL}/geosite-apple.srs")]),
                        );
                        rule.insert(
                            "16".to_string(),
                            json!([
                                "apple.com",
                                "icloud.com",
                                "icloud-content.com",
                                "apple-dns.net",
                                "cdn-apple.com",
                                "mzstatic.com",
                                "aaplimg.com",
                                "itunes.com",
                                "apple-cloudkit.com",
                                "apple-mapkit.com",
                                "testflight.apple.com",
                            ]),
                        );
                    }
                    "windows" => {
                        rule.insert(
                            "5".to_string(),
                            json!([format!("{GEOSITE_URL}/geosite-microsoft.srs")]),
                        );
                        rule.insert(
                            "16".to_string(),
                            json!([
                                "microsoft.com",
                                "windows.com",
                                "windows.net",
                                "windowsupdate.com",
                                "msftconnecttest.com",
                                "msftncsi.com",
                                "live.com",
                                "office.com",
                                "office.net",
                                "microsoftonline.com",
                                "onedrive.com",
                                "msedge.net",
                            ]),
                        );
                    }
                    "linux" => {
                        rule.insert(
                            "5".to_string(),
                            json!([
                                format!("{GEOSITE_URL}/geosite-ubuntu.srs"),
                                format!("{GEOSITE_URL}/geosite-debian.srs"),
                                format!("{GEOSITE_URL}/geosite-fedora.srs"),
                                format!("{GEOSITE_URL}/geosite-archlinux.srs"),
                            ]),
                        );
                        rule.insert(
                            "16".to_string(),
                            json!([
                                "ubuntu.com",
                                "debian.org",
                                "fedoraproject.org",
                                "archlinux.org",
                                "flathub.org",
                                "snapcraft.io",
                            ]),
                        );
                    }
                    // android, fuchsia, or default mobile
                    _ => {
                        rule.insert(
                            "5".to_string(),
                            json!([
                                format!("{GEOSITE_URL}/geosite-android.srs"),
                                format!("{GEOSITE_URL}/geosite-googlefcm.srs"),
                            ]),
                        );
                        rule.insert("16".to_string(), json!(["android.com", "mtalk.google.com"]));
                    }
                }
            }
            Self::BlockAds => {
                rule.insert("3".to_string(), json!(self.key()));
                rule.insert("4".to_string(), json!(OUTBOUND_BLOCK));
                rule.insert(
                    "5".to_string(),
                    json!([
                        format!("{GEO_URL}/block/geosite-category-ads-all.srs"),
                        format!("{GEO_URL}/block/geosite-malware.srs"),
                        format!("{GEO_URL}/block/geosite-phishing.srs"),
                        format!("{GEO_URL}/block/geosite-cryptominers.srs"),
                        format!("{GEO_URL}/block/geoip-malware.srs"),
                        format!("{GEO_URL}/block/geoip-phishing.srs"),
                    ]),
                );
            }
            Self::BypassRegion => {
                rule.insert("3".to_string(), json!(self.key()));
                rule.insert("4".to_string(), json!(OUTBOUND_DIRECT));
                rule.insert(
                    "5".to_string(),
                    json!([
                        format!("{GEO_URL}/country/geosite-{{region}}.srs"),
                        format!("{GEO_URL}/country/geoip-{{region}}.srs"),
                    ]),
                );
            }
        }
        rule
    }
}

/// Returns a single `Rule` JSON string (for `Rule.fromJson(...)`) for the given built-in rule kind & platform.
pub fn builtin_rule_pb_json(short_name: &str, platform: &str) -> Option<String> {
    let kind = BuiltinRuleKind::from_short_name(short_name)?;
    serde_json::to_string(&Value::Object(kind.build_pb_map(platform))).ok()
}

/// Resolves a rule's `name` to its canonical built-in short name (`"bypassLan"`, `"systemDirect"`, ...),
/// including legacy `"builtin:appleDirect"` -> `"systemDirect"`.
pub fn builtin_of_rule_name(rule_name: &str) -> Option<&'static str> {
    BuiltinRuleKind::from_rule_name(rule_name).map(BuiltinRuleKind::short_name)
}

fn extract_rules_array(route_rule_pb_json: &str) -> Result<Vec<Map<String, Value>>, ConfigError> {
    if route_rule_pb_json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root: Value = serde_json::from_str(route_rule_pb_json)?;
    let Some(arr) = root.get("1").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(obj) = item.as_object() {
            out.push(obj.clone());
        }
    }
    Ok(out)
}

fn encode_rules_array(mut rules: Vec<Map<String, Value>>) -> Result<String, ConfigError> {
    for (idx, rule) in rules.iter_mut().enumerate() {
        rule.insert("1".to_string(), json!(idx));
    }
    let mut root = Map::new();
    if !rules.is_empty() {
        root.insert(
            "1".to_string(),
            Value::Array(rules.into_iter().map(Value::Object).collect()),
        );
    }
    Ok(serde_json::to_string(&Value::Object(root))?)
}

/// Mirrors `RulesNotifier._withBuiltins(saved)` in `rules_notifier.dart`:
/// - Replaces built-in rules' content with the app's canonical definition for `platform`, keeping `enabled` and order.
/// - Deduplicates repeated built-ins, keeping the first occurrence.
/// - Prepends missing built-ins in default order with `enabled_by_default`.
/// - Excludes `bypassRegion` when `region == "other"`.
/// - Renumbers `list_order` (`"1"`) `0..n`.
pub fn with_builtins_pb_json(
    saved_route_rule_pb_json: &str,
    region: &str,
    platform: &str,
) -> Result<String, ConfigError> {
    let saved = extract_rules_array(saved_route_rule_pb_json)?;
    let is_other_region = region.eq_ignore_ascii_case("other");

    let mut missing: Vec<BuiltinRuleKind> = BuiltinRuleKind::ALL
        .into_iter()
        .filter(|&b| b != BuiltinRuleKind::BypassRegion || !is_other_region)
        .collect();

    let mut rules: Vec<Map<String, Value>> = Vec::new();
    for rule in saved {
        let name = rule.get("3").and_then(Value::as_str).unwrap_or("");
        match BuiltinRuleKind::from_rule_name(name) {
            None => rules.push(rule),
            Some(builtin) => {
                if let Some(pos) = missing.iter().position(|&m| m == builtin) {
                    missing.remove(pos);
                    let enabled = rule.get("2").and_then(Value::as_bool).unwrap_or(false);
                    let mut fresh = builtin.build_pb_map(platform);
                    fresh.insert("2".to_string(), json!(enabled));
                    rules.push(fresh);
                }
            }
        }
    }

    let mut combined = Vec::with_capacity(missing.len() + rules.len());
    for builtin in missing {
        let mut fresh = builtin.build_pb_map(platform);
        fresh.insert("2".to_string(), json!(builtin.enabled_by_default()));
        combined.push(fresh);
    }
    combined.extend(rules);

    encode_rules_array(combined)
}

/// Mirrors `RulesNotifier.duplicateRule(listOrder)` in `rules_notifier.dart`.
/// Returns `(updated_route_rule_pb_json, new_copy_order)`.
pub fn duplicate_rule_pb_json(
    current_route_rule_pb_json: &str,
    list_order: i64,
) -> Result<(String, i64), ConfigError> {
    let mut current = extract_rules_array(current_route_rule_pb_json)?;
    let found_idx = current.iter().position(|r| {
        r.get("1")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse::<i64>().ok()))
            .unwrap_or(0)
            == list_order
    });
    let Some(idx) = found_idx else {
        return Ok((encode_rules_array(current)?, list_order));
    };

    let mut copy = current[idx].clone();
    let src_name = copy.get("3").and_then(Value::as_str).unwrap_or("");
    let new_name = match BuiltinRuleKind::from_rule_name(src_name) {
        Some(builtin) => builtin.short_name().to_string(),
        None => format!("{src_name} (2)"),
    };
    copy.insert("3".to_string(), json!(new_name));
    copy.insert("2".to_string(), json!(false));

    let insert_at = idx + 1;
    current.insert(insert_at, copy);
    Ok((encode_rules_array(current)?, insert_at as i64))
}
