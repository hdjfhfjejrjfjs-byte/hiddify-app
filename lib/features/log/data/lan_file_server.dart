import 'dart:async';
import 'dart:io';

import 'package:hiddify/core/rust/hiddify_rust.dart';

/// Native Rust LAN HTTP file download server (`hiddify-net::lan_server`) for sharing logs or
/// diagnostics archives from an Android TV to a phone via QR code.
class LanFileServer {
  LanFileServer._(this._handle, this.url);

  final int _handle;
  final Uri url;
  final _downloads = StreamController<InternetAddress>.broadcast();
  Timer? _pollTimer;
  var _closed = false;

  static Future<LanFileServer> start(InternetAddress address, File file) async {
    final info = hiddifyRust.require(
      hiddifyRust.lanFileServerStart(bindIp: address.address, filePath: file.path),
      'lanFileServerStart',
    );
    final server = LanFileServer._(info.handle, Uri.parse(info.url));
    server._pollTimer = Timer.periodic(const Duration(milliseconds: 20), (_) => server._drain());
    return server;
  }

  void _drain() {
    if (_closed) return;
    final ips = hiddifyRust.lanFileServerPoll(_handle);
    if (ips == null || ips.isEmpty) return;
    for (final ip in ips) {
      if (!_downloads.isClosed) {
        _downloads.add(InternetAddress(ip));
      }
    }
  }

  /// The device of each download that went through to the end.
  Stream<InternetAddress> get downloads => _downloads.stream;

  Future<void> close() async {
    if (_closed) return;
    _drain();
    _closed = true;
    _pollTimer?.cancel();
    _pollTimer = null;
    hiddifyRust.lanFileServerClose(_handle);
    await _downloads.close();
  }
}
