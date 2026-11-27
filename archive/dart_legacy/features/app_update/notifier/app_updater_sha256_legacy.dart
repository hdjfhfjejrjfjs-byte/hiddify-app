// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/app_update/notifier/app_updater.dart (SHA-256 digest validation & file hashing)
// Replaced by:   rust/crates/hiddify-crypto (is_valid_sha256_digest, verify_file_sha256)

import 'dart:io';

import 'package:crypto/crypto.dart';

bool legacyIsValidSha256Digest(String digest) =>
    digest.length == 71 && RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(digest);

Future<bool> legacyVerifyFileSha256(File file, int expectedSize, String digest) async {
  try {
    if (expectedSize > 0 && await file.length() != expectedSize) return false;
    final actual = await sha256.bind(file.openRead()).first;
    return actual.toString() == digest.substring(7).toLowerCase();
  } on FileSystemException {
    return false;
  }
}
