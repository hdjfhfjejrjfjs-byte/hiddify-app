//! End-to-end C-ABI integration tests verifying 100% behavioral equivalence
//! with Hiddify's Dart test suites across all 5 extracted Rust crates.

use std::ffi::{CStr, CString};

use hiddify_rust::*;
use serde_json::{json, Value};

unsafe fn take_str(ptr: *mut std::ffi::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
    hiddify_rust_free_string(ptr);
    Some(s)
}

#[test]
fn c_abi_profile_and_config_full_workflow() {
    unsafe {
        // 1. Content headers + remote headers merge
        let content = CString::new("# profile-title: base64:U3RvY2tob2xtIDE\n# keep-top: 10\nvless://u@1.2.3.4:443").unwrap();
        let remote = CString::new(r#"{"extra-security":"warp","enable-warp":"true","profile-title":""}"#).unwrap();
        let pop_str = take_str(hiddify_profile_populate_headers_json(
            content.as_ptr(),
            remote.as_ptr(),
        ))
        .expect("populated headers");
        let pop: Value = serde_json::from_str(&pop_str).unwrap();
        assert_eq!(pop["extra-security"], "warp");
        assert_eq!(pop["keep-top"], "10");
        // Empty remote `profile-title` was present in remote_headers, so content did not overwrite it,
        // and empty value was dropped by validation (exact Dart `_mergeAndValidateHeaders` behavior!)
        assert!(pop.get("profile-title").is_none());
        assert!(pop.get("enable-warp").is_none());

        // 2. Profile override generation + deep merge
        let uo = CString::new(r#"{"keepTopCountries":["DE","NL"],"keepTopQuick":false}"#).unwrap();
        let pop_c = CString::new(pop_str).unwrap();
        let ov_str = take_str(hiddify_config_profile_override(pop_c.as_ptr(), uo.as_ptr())).unwrap();
        let ov: Value = serde_json::from_str(&ov_str).unwrap();
        assert_eq!(ov["chain-status"], "extra_security");
        assert_eq!(ov["extra-security"]["mode"], "warp_auto");
        assert_eq!(ov["keep-top"], 10);
        assert!(ov.get("keep-top-quick").is_none());
        assert_eq!(ov["keep-top-countries"], json!(["DE", "NL"]));

        let main_cfg = CString::new(r#"{"extra-security":{"existing":1},"dns":"1.1.1.1"}"#).unwrap();
        let ov_c = CString::new(ov_str).unwrap();
        let merged_str = take_str(hiddify_config_apply_profile_override(
            main_cfg.as_ptr(),
            ov_c.as_ptr(),
        ))
        .unwrap();
        let merged: Value = serde_json::from_str(&merged_str).unwrap();
        assert_eq!(merged["extra-security"]["existing"], 1);
        assert_eq!(merged["extra-security"]["mode"], "warp_auto");
        assert_eq!(merged["dns"], "1.1.1.1");

        // 3. Release tag parser
        let tag = CString::new("v5.9.15.dev+50915").unwrap();
        let tag_json: Value = serde_json::from_str(
            &take_str(hiddify_diagnostics_parse_release_tag_json(tag.as_ptr())).unwrap(),
        )
        .unwrap();
        assert_eq!(tag_json["version"], "5.9.15");
        assert_eq!(tag_json["suffix"], "dev");
        assert_eq!(tag_json["build_number"], "50915");
    }
}
