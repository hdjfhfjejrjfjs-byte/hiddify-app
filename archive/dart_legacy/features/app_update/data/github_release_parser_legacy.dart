// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/app_update/data/github_release_parser.dart (tag & changelog parsing)
// Replaced by:   rust/crates/hiddify-diagnostics (parse_release_tag, extract_changelog)

abstract class LegacyGithubReleaseParser {
  static final _tag = RegExp(r'^v?(\d+\.\d+\.\d+)(?:[-.]([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$');

  static ({String version, String suffix, String buildNumber})? parseTag(String fullTag) {
    final match = _tag.firstMatch(fullTag.trim());
    if (match == null) return null;
    return (
      version: match.group(1)!,
      suffix: match.group(2) ?? '',
      buildNumber: match.group(3) ?? '',
    );
  }

  static final _changelogHeading = RegExp(
    r"^(#{1,6})\s*(?:change\s*-?\s*logs?|changes|what'?s\s+new|highlights?|تغییرات)\s*:?\s*#*\s*$",
    caseSensitive: false,
  );
  static final _heading = RegExp(r'^(#{1,6})\s');
  static final _downloads = RegExp('download', caseSensitive: false);
  static final _comment = RegExp('<!--.*?-->', dotAll: true);
  static final _rule = RegExp(r'^(?:-{3,}|\*{3,}|_{3,})$');

  static const maxChangelogLength = 4000;

  static String? changelog(String body) {
    final lines = body.replaceAll('\r\n', '\n').replaceAll(_comment, '').split('\n');
    final start = lines.indexWhere((l) => _changelogHeading.hasMatch(l.trim()));
    if (start < 0) return null;
    final level = _changelogHeading.firstMatch(lines[start].trim())!.group(1)!.length;
    final section = <String>[];
    for (final line in lines.skip(start + 1)) {
      final trimmed = line.trim();
      final heading = _heading.firstMatch(trimmed);
      if (heading != null && (heading.group(1)!.length <= level || _downloads.hasMatch(trimmed))) break;
      if (_rule.hasMatch(trimmed) || trimmed.startsWith('<table')) break;
      section.add(line);
    }
    var text = section.join('\n').trim();
    if (text.isEmpty) return null;
    if (text.length > maxChangelogLength) {
      final cut = text.lastIndexOf('\n', maxChangelogLength);
      text = '${text.substring(0, cut > 0 ? cut : maxChangelogLength).trimRight()}\n\n…';
    }
    return text;
  }
}
