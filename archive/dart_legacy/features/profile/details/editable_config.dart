// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/profile/details/profile_details_notifier.dart (_editableConfig)
// Replaced by:   rust/crates/hiddify-config (editable_config)

import 'dart:convert';

({String text, bool isJson}) legacyEditableConfig(String content) {
  try {
    final jsonObject = jsonDecode(content);
    final List<Map<String, dynamic>> outbounds = [];
    if (jsonObject is Map<String, dynamic> && jsonObject['outbounds'] is List) {
      for (final outbound in jsonObject['outbounds'] as List<dynamic>) {
        if (outbound is Map<String, dynamic> &&
            outbound['type'] != null &&
            !['selector', 'urltest', 'dns', 'block'].contains(outbound['type']) &&
            !['direct', 'bypass', 'direct-fragment'].contains(outbound['tag'])) {
          outbounds.add(outbound);
        }
      }
    }
    // ignore: avoid_dynamic_calls
    final endpoints = jsonObject['endpoints'] as List? ?? [];
    return (text: '{"outbounds": ${json.encode(outbounds)},"endpoints":${json.encode(endpoints)} }', isJson: true);
  } catch (_) {
    return (text: content, isJson: false);
  }
}
