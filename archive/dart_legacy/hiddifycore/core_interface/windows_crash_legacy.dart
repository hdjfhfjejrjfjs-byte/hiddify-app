// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/hiddifycore/core_interface/windows_crash.dart (applicationErrorFor & describeExitCode)
// Replaced by:   rust/crates/hiddify-diagnostics (core_guard::windows_application_error_for, describe_windows_exit_code)

String? legacyApplicationErrorFor(String xml, int pid) {
  for (final event in RegExp('<Event[ >].*?</Event>', dotAll: true).allMatches(xml)) {
    final data = RegExp(
      r'<Data(?:\s[^>]*)?>(.*?)</Data>',
      dotAll: true,
    ).allMatches(event.group(0)!).map((match) => match.group(1)!.trim()).toList();
    if (data.length < 9) continue;
    final eventPid = int.tryParse(data[8].replaceFirst(RegExp('^0[xX]'), ''), radix: 16);
    if (eventPid != pid) continue;
    final code = int.tryParse(data[6].replaceFirst(RegExp('^0[xX]'), ''), radix: 16);
    final name = code == null ? null : _ntStatusNames[code];
    final codeText = code == null ? data[6] : '0x${code.toRadixString(16).padLeft(8, '0')}';
    return 'a native crash in ${data[3]} (exception $codeText${name == null ? '' : ', $name'} at offset ${data[7]})';
  }
  return null;
}

String? legacyDescribeExitCode(int exitCode) {
  final code = exitCode & 0xFFFFFFFF;
  final name = _ntStatusNames[code];
  if (name != null) return 'ended by Windows: $name (0x${code.toRadixString(16)})';
  if (code == 1) return 'ended with exit code 1: an error, or another program (Task Manager, an antivirus) ended it';
  return null;
}

const _ntStatusNames = {
  0xC0000005: 'access violation',
  0xC0000409: 'a fail-fast exit or a stack buffer overrun',
  0xC00000FD: 'stack overflow',
  0xC0000374: 'heap corruption',
  0xC0000017: 'out of memory',
  0xC000009A: 'out of system resources',
  0xC0000135: 'a DLL it needs was not found',
  0xC0000142: 'a DLL failed to initialize',
  0xC000001D: 'an illegal instruction',
  0xC0000094: 'an integer division by zero',
  0xC000013A: 'ended with Ctrl+C or its console closing',
  0x40010004: 'ended by a debugger or with the system shutting down',
  0xE06D7363: 'an unhandled C++ exception',
};
