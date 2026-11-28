import 'dart:isolate';

import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/hiddifycore/core_interface/admin_permission_refused.dart';
import 'package:hiddify/utils/custom_loggers.dart';

final _logger = infraLogger('AppImageService');

/// Native Rust-managed AppImage foreground VPN service (`hiddify-net::admin_service`).
class AppImageService {
  AppImageService._(this._handle, this.secret);

  static const port = 18021;

  final int _handle;
  final String secret;
  bool _stopped = false;

  bool get running {
    if (_stopped) return false;
    return hiddifyRust.appImageServiceIsRunning(_handle) ?? false;
  }

  /// Asks for administrator rights (`pkexec`, or `sudo` with an askpass dialog) in Rust and
  /// starts `<appImage> HiddifyCli core-service`; returns once `HIDDIFY_SERVICE_READY` arrives.
  static Future<AppImageService> start(String appImage, int ownerPid) async {
    final outcome = await Isolate.run(
      () => hiddifyRust.require(
        hiddifyRust.appImageServiceStart(appImagePath: appImage, ownerPid: ownerPid),
        'appImageServiceStart',
      ),
    );
    switch (outcome.status) {
      case 'ok':
        _logger.info('AppImage VPN service (handle ${outcome.handle}) started on port $port');
        return AppImageService._(outcome.handle, outcome.secret);
      case 'no_admin_prompt':
        throw VpnServiceFailure.noAdminPrompt;
      case 'no_admin_agent':
        throw VpnServiceFailure.noAdminAgent;
      case 'permission_refused':
        throw const AdminPermissionRefused();
      case 'core_error':
        throw StateError(outcome.message ?? 'AppImage core error');
      default:
        throw VpnServiceFailure(outcome.message ?? 'The VPN service did not start.');
    }
  }

  /// Closes the service's stdin so its engine restores TUN/routes and exits.
  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    final handle = _handle;
    await Isolate.run(() => hiddifyRust.appImageServiceStop(handle));
  }
}
