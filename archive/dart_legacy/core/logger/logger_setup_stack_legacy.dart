// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/core/logger/logger_setup.dart (originOf & _ownFrames)
// Replaced by:   rust/crates/hiddify-diagnostics (log_crash_store::origin_of_stack, own_stack_frames)

bool _isOurs(String frame, String ownPackagePrefix, String loggerInternals) =>
    frame.contains(ownPackagePrefix) && !frame.contains(loggerInternals);

String? legacyOriginOf(
  StackTrace? stack, {
  String ownPackagePrefix = 'package:hiddify/',
  String loggerInternals = 'package:hiddify/core/logger/',
}) {
  if (stack == null) return null;

  for (final frame in stack.toString().split('\n')) {
    if (!_isOurs(frame, ownPackagePrefix, loggerInternals)) continue;

    final open = frame.indexOf('(');
    final close = frame.lastIndexOf(')');
    if (open == -1 || close <= open) continue;

    final method = frame
        .substring(0, open)
        .replaceFirst(RegExp(r'^#\d+\s+'), '')
        .replaceAll('.<anonymous closure>', '')
        .trim();

    final parts = frame.substring(open + 1, close).split(':');
    if (parts.length < 3) continue;
    final file = parts[parts.length - 3].split('/').last;
    final line = parts[parts.length - 2];

    return '$method ($file:$line)';
  }
  return null;
}

StackTrace? legacyOwnFrames(
  StackTrace? stack, {
  String ownPackagePrefix = 'package:hiddify/',
  String loggerInternals = 'package:hiddify/core/logger/',
  int errorFrames = 4,
}) {
  if (stack == null) return null;
  final mine = stack
      .toString()
      .split('\n')
      .where((f) => _isOurs(f, ownPackagePrefix, loggerInternals))
      .take(errorFrames)
      .toList();
  if (mine.isEmpty) return null;
  return StackTrace.fromString(mine.join('\n'));
}
