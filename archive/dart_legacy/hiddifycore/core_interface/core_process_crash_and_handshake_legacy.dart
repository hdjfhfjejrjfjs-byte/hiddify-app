// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/hiddifycore/core_interface/core_process.dart (stdout handshake, crashReasonOf, crashHeadline)
// Replaced by:   rust/crates/hiddify-diagnostics (core_guard::parse_core_stdout_line, core_crash_headline, core_crash_reason_of)

import 'dart:convert';
import 'dart:io';

const _maxReasonLength = 300;

Future<String?> legacyCrashReasonOf(File crashFile, List<String> stderrTail, DateTime startedAt) async {
  String? fromFile;
  try {
    if (await crashFile.exists() &&
        !(await crashFile.lastModified()).isBefore(startedAt.subtract(const Duration(seconds: 1)))) {
      fromFile = legacyCrashHeadline(await crashFile.readAsString());
    }
  } catch (_) {}
  return fromFile ?? legacyCrashHeadline(stderrTail.join('\n'));
}

String? legacyCrashHeadline(String output) {
  final lines = const LineSplitter().convert(output).map((line) => line.trim()).toList();
  final line =
      lines.where((line) => line.startsWith('panic: ') || line.startsWith('fatal error: ')).firstOrNull ??
      lines.where((line) => RegExp('^SIG[A-Z]+: ').hasMatch(line)).firstOrNull ??
      lines.where((line) => RegExp('^Exception 0x[0-9a-fA-F]+').hasMatch(line)).firstOrNull;
  if (line == null) return null;
  return line.length > _maxReasonLength ? '${line.substring(0, _maxReasonLength)}…' : line;
}
