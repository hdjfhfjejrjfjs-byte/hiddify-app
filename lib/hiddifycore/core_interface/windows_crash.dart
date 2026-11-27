import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';

/// Why a core process ended on Windows when it left no Go crash output: a native crash (in a DLL
/// such as cronet or wintun, on a thread of its own, which Go's handler does not see) or another
/// program ending it. Windows records a native crash as an "Application Error" event (1000) with
/// the faulting module, the exception code and the process id; the exit code alone names what
/// ended it. Before, the app said only "exit code -1073741819" (hiddify-app#2517).
class WindowsCrash {
  WindowsCrash._();

  /// The reason for process [pid] that ended with [exitCode]: the Application Error event of that
  /// process when Windows recorded one, else what the exit code means, or null.
  static Future<String?> reason(int pid, int exitCode) async {
    // Windows writes the event a moment after the process is gone
    for (var attempt = 0; attempt < 4; attempt++) {
      final event = await _applicationErrorOf(pid);
      if (event != null) return event;
      await Future<void>.delayed(const Duration(milliseconds: 500));
    }
    return describeExitCode(exitCode);
  }

  static Future<String?> _applicationErrorOf(int pid) async {
    try {
      final result = await Process.run('wevtutil', [
        'qe',
        'Application',
        '/q:*[System[Provider[@Name="Application Error"] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= 120000]]]',
        '/f:xml',
        '/rd:true',
        '/c:10',
      ]).timeout(const Duration(seconds: 5));
      if (result.exitCode != 0) return null;
      return applicationErrorFor(result.stdout as String, pid);
    } catch (_) {
      return null;
    }
  }

  /// The crash of process [pid] in the Application Error events in [xml] (wevtutil's /f:xml), or
  /// null. The event's data is read by position, which does not depend on the system's language:
  /// application, its version and time stamp, the faulting module, its version and time stamp,
  /// the exception code, the fault offset, the process id (hex).
  @visibleForTesting
  static String? applicationErrorFor(String xml, int pid) {
    hiddifyRust.ensureAvailable('windowsAppErrorFor');
    return hiddifyRust.windowsAppErrorFor(xml, pid);
  }

  /// What a Windows exit code says ended the process, or null for an ordinary code.
  @visibleForTesting
  static String? describeExitCode(int exitCode) {
    hiddifyRust.ensureAvailable('describeWindowsExitCode');
    return hiddifyRust.describeWindowsExitCode(exitCode);
  }
}
