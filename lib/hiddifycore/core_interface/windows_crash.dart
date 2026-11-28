import 'dart:async';
import 'dart:isolate';

import 'package:flutter/foundation.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';

/// Why a core process ended on Windows when it left no Go crash output, delegated to
/// `hiddify-net::core_process::windows_crash_reason` and `hiddify-diagnostics`.
class WindowsCrash {
  WindowsCrash._();

  /// The reason for process [pid] that ended with [exitCode].
  static Future<String?> reason(int pid, int exitCode) {
    hiddifyRust.ensureAvailable('windowsCrashReason');
    return Isolate.run(() => hiddifyRust.windowsCrashReason(pid, exitCode));
  }

  @visibleForTesting
  static String? applicationErrorFor(String xml, int pid) {
    hiddifyRust.ensureAvailable('windowsAppErrorFor');
    return hiddifyRust.windowsAppErrorFor(xml, pid);
  }

  @visibleForTesting
  static String? describeExitCode(int exitCode) {
    hiddifyRust.ensureAvailable('describeWindowsExitCode');
    return hiddifyRust.describeWindowsExitCode(exitCode);
  }
}
