// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/crash_report/data/crash_excerpt.dart (_goCrash & _excerpt)
// Replaced by:   rust/crates/hiddify-diagnostics (crash_excerpt)

typedef CrashExcerpt = ({String headline, String text});

const _maxText = 3000;
const _maxLines = 60;

final _goCrash = RegExp(r'^(panic: |fatal error: |runtime: |SIG[A-Z]+: |\[signal )', multiLine: true);

CrashExcerpt? legacyExtractCrashExcerpt(String content, {required bool findGoMarker}) {
  final start = findGoMarker ? (_goCrash.firstMatch(content)?.start ?? 0) : 0;
  return _excerpt(content, start);
}

CrashExcerpt? _excerpt(String content, int start) {
  final lines = content.substring(start).split('\n');
  final headline = lines.firstWhere((line) => line.trim().isNotEmpty, orElse: () => '').trim();
  if (headline.isEmpty) return null;
  var text = lines.take(_maxLines).join('\n').trimRight();
  if (text.length > _maxText) text = '${text.substring(0, _maxText)}\n…';
  return (headline: headline.length > 200 ? '${headline.substring(0, 200)}…' : headline, text: text);
}
