// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/hiddifycore/grpc_errors.dart, hiddify_core_service.dart (_startFailure), core_interface_desktop.dart (generateRandomPassword)
// Replaced by:   rust/crates/hiddify-diagnostics (core_guard::is_core_gone_grpc_error, classify_core_start_failure) & rust/crates/hiddify-crypto (generate_alphanumeric_secret)

import 'dart:math';

import 'package:grpc/grpc.dart';

bool legacyIsCoreGoneError(Object? error) {
  if (error is! GrpcError) return false;
  if (error.code == StatusCode.unavailable) return true;
  return error.code == StatusCode.unknown &&
      (error.message ?? '').contains('Connection is being forcefully terminated');
}

String legacyGenerateRandomPassword(int length) {
  const characters = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
  final random = Random();
  return List.generate(length, (_) => characters[random.nextInt(characters.length)]).join();
}
