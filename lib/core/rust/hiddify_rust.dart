import 'package:hiddify/core/rust/hiddify_rust_stub.dart'
    if (dart.library.ffi) 'package:hiddify/core/rust/hiddify_rust_ffi.dart';

export 'package:hiddify/core/rust/hiddify_rust_stub.dart'
    if (dart.library.ffi) 'package:hiddify/core/rust/hiddify_rust_ffi.dart'
    show HiddifyRustNative;

/// Global singleton entry point to the cross-platform Rust domain modules (`libhiddify_rust`).
///
/// On iOS, Android, macOS, Linux, and Windows builds with the compiled native library,
/// callers execute native Rust directly via `dart:ffi`.
HiddifyRustNative get hiddifyRust => HiddifyRustNative.instance;

extension HiddifyRustRequired on HiddifyRustNative {
  /// Returns [value] from a native Rust call, or throws a [StateError] when `libhiddify_rust` is unavailable.
  T require<T>(T? value, String operation) {
    if (value != null) return value;
    throw StateError('hiddify_rust native library is unavailable for $operation.');
  }

  /// Ensures `libhiddify_rust` is loaded before invoking a Rust operation that may legitimately return `null`.
  void ensureAvailable(String operation) {
    if (!isAvailable) {
      throw StateError('hiddify_rust native library is unavailable for $operation.');
    }
  }
}
