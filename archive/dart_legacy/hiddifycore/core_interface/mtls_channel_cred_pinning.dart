// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/hiddifycore/core_interface/mtls_channel_cred.dart (pinning helpers & onBadCertificate fallback)
// Replaced by:   rust/crates/hiddify-crypto (verify_pinned_cert)

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

bool legacyVerifyPinnedCert({
  required X509Certificate cert,
  required String host,
  required Uint8List serverCertificateDer,
  required String serverPemBody,
}) {
  if (!_isLoopbackHost(host)) return false;
  // Prefer exact DER pin; fall back to PEM body match (macOS sometimes
  // re-encodes the presented cert so raw DER bytes differ).
  if (_bytesEqual(cert.der, serverCertificateDer)) return true;
  final presented = _pemBody(cert.pem);
  return presented.isNotEmpty && presented == serverPemBody;
}

bool _isLoopbackHost(String host) {
  // grpc-dart may pass "127.0.0.1:port" or just "127.0.0.1".
  final name = host.split(':').first.replaceAll('[', '').replaceAll(']', '');
  return name == '127.0.0.1' || name == 'localhost' || name == '::1';
}

bool _bytesEqual(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

String _pemBody(String pem) {
  return pem
      .replaceAll('-----BEGIN CERTIFICATE-----', '')
      .replaceAll('-----END CERTIFICATE-----', '')
      .replaceAll(RegExp(r'\s'), '');
}

Uint8List _pemToDer(String pem) {
  return Uint8List.fromList(base64.decode(_pemBody(pem)));
}
