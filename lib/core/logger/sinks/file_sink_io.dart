import 'package:hiddify/core/rust/hiddify_rust.dart';

bool get isLogFileOpen => hiddifyRust.require(hiddifyRust.isLogFileOpen(), 'isLogFileOpen');

/// Opens by creating the file and proving it is writable, so a bad path is
/// known now rather than silently swallowed at the first error worth recording.
///
/// It spans runs, so it is cut back to its newest part at the start of one
/// once it grows past 1 MiB: on a phone too, where it is kept for the
/// run a crash ended.
void openLogFile(String path) {
  hiddifyRust.require(hiddifyRust.openLogFile(path), 'openLogFile');
}

/// Written straight through, no batching.
///
/// Only errors reach this file, and they are rare, so a syscall each is cheap
/// and nothing can be sitting in memory when the crash that follows takes the
/// app down.
void writeLogLine(String line) {
  hiddifyRust.ensureAvailable('writeLogLine');
  hiddifyRust.writeLogLine(line);
}

Future<void> closeLogFile() async {
  hiddifyRust.ensureAvailable('closeLogFile');
  hiddifyRust.closeLogFile();
}

/// Writes [lines] to [path] in one go, replacing whatever was there. Used by
/// the logs page to export the in-memory history, which is a different file.
Future<void> writeLinesToFile(String path, Iterable<String> lines) async {
  hiddifyRust.require(hiddifyRust.writeTextToFile(path, lines.join('\n')), 'writeTextToFile');
}
