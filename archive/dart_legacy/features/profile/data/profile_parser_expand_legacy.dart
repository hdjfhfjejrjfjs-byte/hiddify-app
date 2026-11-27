// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/features/profile/data/profile_parser.dart (_flattenHeaders, expandLinks, extraSecurityMode, keepTopHeader, fragmentTricks)
// Replaced by:   rust/crates/hiddify-profile (download) & rust/crates/hiddify-config

import 'package:hiddify/core/model/optional_range.dart';
import 'package:hiddify/singbox/model/singbox_config_enum.dart';

Map<String, dynamic> legacyFlattenHeaders(Map<String, List<String>> headers) =>
    headers.map((key, value) => MapEntry(key, value.length == 1 ? value.first : value));

ChainMode? legacyExtraSecurityMode(String? value) {
  if (value == null) return null;
  final trimmed = value.trim();
  for (final mode in ChainMode.values) {
    if (mode.isProfile()) continue;
    if (mode.key == trimmed || (mode.isWarp() && trimmed == 'warp')) return mode;
  }
  return null;
}

int? legacyKeepTopHeader(String? value) {
  final keepTop = int.tryParse(value?.trim() ?? '');
  if (keepTop == null || (keepTop < 1 && keepTop != -1)) return null;
  return keepTop;
}

Map<String, dynamic>? legacyFragmentTricks(String? value) {
  if (value == null) return null;
  final parts = value.split(',').map((e) => e.trim()).toList();
  if (parts.length < 3) return null;
  final leadingMethod = parts.first.toLowerCase() == 'tlshello';
  final size = leadingMethod ? parts[1] : parts[0];
  final sleep = leadingMethod ? parts[2] : parts[1];
  if (OptionalRange.tryParse(size) == null || OptionalRange.tryParse(sleep) == null) return null;
  return {'enable-fragment': true, 'fragment-size': size, 'fragment-sleep': sleep};
}
