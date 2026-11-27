// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/features/crash_report/data/app_diagnostics.dart (_preferenceValue) & diagnostics_exporter.dart (byte-budget assembly)
// Replaced by:   rust/crates/hiddify-diagnostics (log_crash_store::scrub_preferences_map, assemble_diagnostics_payload)

final _hiddenPreference = RegExp('password|secret|license|token|pairing|clean-ip|per_app_proxy_(include|exclude)_list');
const _maxPreferenceLength = 4000;

Object? legacyPreferenceValue(String key, Object? value) {
  if (_hiddenPreference.hasMatch(key)) {
    return switch (value) {
      null => null,
      final List<Object?> list => '<${list.length} entries>',
      final String text when key.endsWith('_list') => '<${text.split(';').where((e) => e.isNotEmpty).length} entries>',
      final String text when text.isEmpty => '<empty>',
      _ => '<set>',
    };
  }
  if (value is String && value.length > _maxPreferenceLength) {
    return '${value.substring(0, _maxPreferenceLength)}… <${value.length} characters>';
  }
  return value;
}

List<int> legacyTailBytes(List<int> content, int max) =>
    content.length > max ? content.sublist(content.length - max) : content;
