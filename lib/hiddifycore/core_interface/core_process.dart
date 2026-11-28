import 'dart:async';
import 'dart:io';
import 'dart:isolate';

import 'package:flutter/foundation.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hiddify/utils/custom_loggers.dart';

final _logger = infraLogger('CoreProcess');

/// Why the core's process could not start: its own error, or that it exited first.
class CoreProcessException implements Exception {
  const CoreProcessException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// The desktop core in a process of its own, supervised natively by `hiddify-net::core_process`
/// via `libhiddify_rust` (C-ABI v7).
class CoreProcess {
  CoreProcess._(this._handle, this.processId, this.serverCertificatePem);

  static const _readyTimeout = Duration(seconds: 60);

  final int _handle;

  /// Operating system PID of the core child process.
  final int processId;

  /// The gRPC server certificate (PEM), empty without TLS.
  final String serverCertificatePem;

  bool _stopping = false;
  Future<int>? _exitFuture;

  /// Completes with the exit code when the core process exits. [stop] completes it as well.
  Future<int> get exitCode => _exitFuture ??= _waitExit();

  Future<int> _waitExit() {
    final handle = _handle;
    return Isolate.run(() => hiddifyRust.coreProcessWaitExit(handle) ?? -1);
  }

  /// Whether the exit was asked for by [stop].
  bool get stopping => _stopping || (hiddifyRust.coreProcessIsStopping(_handle) ?? false);

  /// The launcher of this build, or null when it has none (tests, or a core built before it).
  static String? executablePath() {
    if (Platform.environment.containsKey('FLUTTER_TEST')) return null;
    hiddifyRust.ensureAvailable('coreProcessExecutablePath');
    return hiddifyRust.coreProcessExecutablePath(Platform.resolvedExecutable);
  }

  /// Starts the core and waits until its gRPC server listens (`HIDDIFY_CORE_READY`).
  static Future<CoreProcess> start({
    required String executable,
    required String baseDir,
    required String workingDir,
    required String tempDir,
    required String listen,
    required int mode,
    required String secret,
    required bool debug,
  }) async {
    hiddifyRust.ensureAvailable('coreProcessStart');
    final modeName = SetupMode.valueOf(mode)?.name ?? '$mode';
    final currentPid = pid;
    final outcome = await Isolate.run(
      () => hiddifyRust.require(
        hiddifyRust.coreProcessStart(
          executable: executable,
          baseDir: baseDir,
          workingDir: workingDir,
          tempDir: tempDir,
          listen: listen,
          mode: mode,
          modeName: modeName,
          secret: secret,
          debug: debug,
          ownerPid: currentPid,
        ),
        'coreProcessStart',
      ),
    );

    switch (outcome.status) {
      case 'ok':
        _logger.info('core process ${outcome.pid} started: $executable');
        return CoreProcess._(outcome.handle, outcome.pid, outcome.certificatePem);
      case 'timeout':
        throw TimeoutException(outcome.message ?? 'waiting for core process ready', _readyTimeout);
      case 'spawn_error':
        throw ProcessException(executable, const ['core'], outcome.message ?? 'failed to spawn core process');
      default:
        throw CoreProcessException(outcome.message ?? 'core error');
    }
  }

  /// Why the core crashed, from its Go crash output or Windows Event 1000 / NTSTATUS exit code.
  Future<String?> crashReason() async {
    hiddifyRust.ensureAvailable('coreProcessCrashReason');
    final handle = _handle;
    final reason = await Isolate.run(() => hiddifyRust.coreProcessCrashReason(handle));
    hiddifyRust.coreProcessForget(handle);
    return reason;
  }

  @visibleForTesting
  static Future<String?> crashReasonOf(File crashFile, List<String> stderrTail, DateTime startedAt) async {
    hiddifyRust.ensureAvailable('coreCrashReasonOf');
    return hiddifyRust.coreCrashReasonOf(crashFile.path, stderrTail, startedAt.millisecondsSinceEpoch);
  }

  static String? crashHeadline(String output) {
    hiddifyRust.ensureAvailable('coreCrashHeadline');
    return hiddifyRust.coreCrashHeadline(output);
  }

  /// Stops the core gracefully (closing `stdin`), escalating to `SIGKILL` after 10s if needed.
  Future<void> stop() async {
    if (_stopping) {
      await exitCode;
      return;
    }
    _stopping = true;
    final handle = _handle;
    await Isolate.run(() => hiddifyRust.coreProcessStop(handle));
    await exitCode;
  }
}
