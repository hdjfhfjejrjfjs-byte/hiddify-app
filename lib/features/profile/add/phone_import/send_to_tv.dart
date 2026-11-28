import 'dart:isolate';

import 'package:hiddify/core/rust/hiddify_rust.dart';

/// How sending a link to a TV went.
enum SendToTvResult {
  sent,

  /// The scanned code is not the one Hiddify shows on a TV.
  notTvCode,

  /// The TV answered, but no longer takes links at this code: its sheet was closed, or locked.
  expired,

  /// The TV has too many links waiting to be imported.
  busy,

  /// The TV could not be reached: another network, or it is gone.
  unreachable,
}

/// Validates a QR code scanned from a TV's add-profile sheet via `hiddify-net::lan_server`.
Uri? parseTvImportCode(String scanned) {
  hiddifyRust.ensureAvailable('parseTvImportCode');
  final normalized = hiddifyRust.parseTvImportCode(scanned);
  return normalized == null ? null : Uri.parse(normalized);
}

/// Sends [link] to the TV page that [scanned] points to via native Rust (`hiddify-net::lan_server`).
Future<SendToTvResult> sendLinkToTv(String scanned, String link) async {
  final outcome = await Isolate.run(
    () => hiddifyRust.require(hiddifyRust.sendLinkToTv(scanned, link), 'sendLinkToTv'),
  );
  return switch (outcome) {
    'sent' => SendToTvResult.sent,
    'notTvCode' => SendToTvResult.notTvCode,
    'expired' => SendToTvResult.expired,
    'busy' => SendToTvResult.busy,
    _ => SendToTvResult.unreachable,
  };
}
