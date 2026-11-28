import 'dart:async';
import 'dart:io';

import 'package:hiddify/core/rust/hiddify_rust.dart';

/// One step of the page's guide.
typedef PhoneImportStep = ({String title, String body});

/// The texts of the page the phone opens, in the TV's language.
typedef PhoneImportPageText = ({
  String lang,
  bool rtl,
  String title,
  String intro,
  PhoneImportStep network,
  PhoneImportStep copy,
  PhoneImportStep paste,
  PhoneImportStep confirm,
  String label,
  String hint,
  String send,
  String sentTitle,
  String sent,
  String empty,
  String footer,
});

/// What a phone sent: a subscription link or a config, waiting for the TV user to import it.
class PhoneEntry {
  PhoneEntry({required this.text, required this.from, required this.at});

  final String text;
  final InternetAddress from;
  final DateTime at;
}

/// Native Rust LAN HTTP server (`hiddify-net::lan_server`) where a phone can send a subscription
/// link to an Android TV on the same local network.
class PhoneImportServer {
  PhoneImportServer._(this._handle, this.url);

  static const tokenLength = 6;
  static const maxMisses = 10;
  static const maxText = 64 * 1024;
  static const maxPending = 20;

  final int _handle;
  final Uri url;
  final _entries = StreamController<PhoneEntry>.broadcast();
  final _locked = Completer<void>();
  Timer? _pollTimer;
  var _closed = false;

  /// Starts serving on [address], which must be this device's address on the local network.
  static Future<PhoneImportServer> start(InternetAddress address, PhoneImportPageText text) async {
    final info = hiddifyRust.require(
      hiddifyRust.phoneImportServerStart(
        bindIp: address.address,
        pageText: {
          'lang': text.lang,
          'rtl': text.rtl,
          'title': text.title,
          'intro': text.intro,
          'network': {'title': text.network.title, 'body': text.network.body},
          'copy': {'title': text.copy.title, 'body': text.copy.body},
          'paste': {'title': text.paste.title, 'body': text.paste.body},
          'confirm': {'title': text.confirm.title, 'body': text.confirm.body},
          'label': text.label,
          'hint': text.hint,
          'send': text.send,
          'sentTitle': text.sentTitle,
          'sent': text.sent,
          'empty': text.empty,
          'footer': text.footer,
        },
      ),
      'phoneImportServerStart',
    );
    final server = PhoneImportServer._(info.handle, Uri.parse(info.url));
    server._startPolling();
    return server;
  }

  void _startPolling() {
    _pollTimer = Timer.periodic(const Duration(milliseconds: 20), (_) => _drain());
  }

  void _drain() {
    if (_closed) return;
    final snap = hiddifyRust.phoneImportServerPoll(_handle);
    if (snap == null) return;
    if (snap.locked && !_locked.isCompleted) {
      _locked.complete();
    }
    for (final ev in snap.entries) {
      if (!_entries.isClosed) {
        _entries.add(
          PhoneEntry(
            text: ev.text,
            from: InternetAddress(ev.fromIp),
            at: DateTime.fromMillisecondsSinceEpoch(ev.timestampMs),
          ),
        );
      }
    }
  }

  /// Each text a phone sends, as it arrives.
  Stream<PhoneEntry> get entries => _entries.stream;

  /// Completes when too many wrong tries locked the page.
  Future<void> get whenLocked => _locked.future;

  bool get isLocked {
    if (_locked.isCompleted) return true;
    if (_closed) return false;
    final lockedNow = hiddifyRust.phoneImportServerIsLocked(_handle) ?? false;
    if (lockedNow && !_locked.isCompleted) {
      _locked.complete();
    }
    return lockedNow;
  }

  /// One entry was imported or discarded on the TV, so there is room for another.
  void handled() {
    if (!_closed) {
      hiddifyRust.phoneImportServerHandled(_handle);
    }
  }

  Future<void> close() async {
    if (_closed) return;
    _drain();
    _closed = true;
    _pollTimer?.cancel();
    _pollTimer = null;
    hiddifyRust.phoneImportServerClose(_handle);
    await _entries.close();
  }
}

/// Whether [address] is on a local network: private IPv4 (10/8, 172.16/12, 192.168/16), link-local,
/// unique-local IPv6 (fc00::/7) or loopback.
bool isLocalNetwork(InternetAddress address) =>
    hiddifyRust.require(hiddifyRust.isLocalNetworkBytes(address.rawAddress), 'isLocalNetworkBytes');
