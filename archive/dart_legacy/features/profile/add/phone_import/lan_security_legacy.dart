// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/features/profile/add/phone_import/phone_import_server.dart & lib/features/log/data/lan_file_server.dart
// Replaced by:   rust/crates/hiddify-net (is_local_network_bytes, is_allowed_host_header, parse_phone_import_form) & rust/crates/hiddify-crypto (generate_numeric_pin, generate_url_token, verify_secret_path)

import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

String legacyNewPhoneImportPin(int tokenLength) {
  final random = Random.secure();
  return List.generate(tokenLength, (_) => random.nextInt(10)).join();
}

String legacyNewLanFileToken() {
  final random = Random.secure();
  return base64Url.encode(List.generate(18, (_) => random.nextInt(256)));
}

bool legacyVerifySecretPath(String path, String token) {
  final expected = '/$token';
  if (path.length != expected.length) return false;
  var diff = 0;
  for (var i = 0; i < path.length; i++) {
    diff |= path.codeUnitAt(i) ^ expected.codeUnitAt(i);
  }
  return diff == 0;
}

bool legacyIsLocalNetwork(InternetAddress address) {
  final ip = address.type == InternetAddressType.IPv6 && _isMappedIPv4(address.rawAddress)
      ? InternetAddress.fromRawAddress(address.rawAddress.sublist(12))
      : address;
  if (ip.isLoopback || ip.isLinkLocal) return true;
  final b = ip.rawAddress;
  if (ip.type == InternetAddressType.IPv4) {
    return b[0] == 10 || (b[0] == 172 && b[1] >= 16 && b[1] <= 31) || (b[0] == 192 && b[1] == 168);
  }
  return (b[0] & 0xfe) == 0xfc;
}

bool _isMappedIPv4(Uint8List b) =>
    b.length == 16 && b.sublist(0, 10).every((x) => x == 0) && b[10] == 0xff && b[11] == 0xff;
