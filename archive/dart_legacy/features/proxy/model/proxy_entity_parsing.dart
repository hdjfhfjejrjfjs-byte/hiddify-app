// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/proxy/model/proxy_entity.dart (tag/status/scanner parsing helpers)
// Replaced by:   rust/crates/hiddify-net (sanitize_tag, strip_status_mark, is_invalid_proxy_type, split_scanner_endpoint)

String legacySanitizedTag(String tag) => tag.replaceFirst(RegExp(r"\§[^]*"), "").trimRight();

final _invalidType = RegExp(r'^Invalid:?\s+\S');
final _scannerEndpoint = RegExp(r'^(.*?) · 🔍 (\[[0-9a-fA-F:]+\]:\d+|\d+\.\d+\.\d+\.\d+:\d+)(.*)$');
final _statusMark = RegExp(r'\s+(⏳|⌛|🔍|❌|🔑)\s.*$');

String legacyStripStatusMark(String type) => type.replaceFirst(_statusMark, '');

bool legacyIsInvalidProxyType(String typeWithoutStatus) => _invalidType.hasMatch(typeWithoutStatus);

({String before, String endpoint, String after})? legacySplitScannerEndpoint(String type) {
  final m = _scannerEndpoint.firstMatch(type);
  return m == null ? null : (before: m[1]!, endpoint: m[2]!, after: m[3]!);
}
