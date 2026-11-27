import 'dart:io';
import 'dart:isolate';

import 'package:hiddify/core/rust/hiddify_rust.dart';

/// The client certificate and private key the app and the platform service present to the local
/// hiddify-core gRPC server (mTLS).
///
/// They are made once and kept in a private folder, so every launch, and the native service
/// (Android), use the same identity: the core knows it by its certificate alone. The key is
/// PKCS#8 PEM, which both Dart and the Android keystore classes read.
class GrpcClientIdentity {
  const GrpcClientIdentity({required this.certificatePem, required this.privateKeyPem});

  final String certificatePem;
  final String privateKeyPem;

  static const certificateFile = 'client.crt';
  static const privateKeyFile = 'client.key';

  /// Reads the identity from [directory], or makes and saves one when there is none, or it is
  /// unreadable or about to expire.
  static Future<GrpcClientIdentity> loadOrCreate(Directory directory) async {
    final dirPath = directory.path;
    final native = await Isolate.run(
      () => hiddifyRust.require(hiddifyRust.loadOrCreateIdentity(dirPath), 'loadOrCreateIdentity'),
    );
    return GrpcClientIdentity(certificatePem: native.certificatePem, privateKeyPem: native.privateKeyPem);
  }

  /// A new RSA identity: RSA matches the Go server's crypto and avoids EC PKCS#8 quirks on iOS.
  factory GrpcClientIdentity.generate() {
    final native = hiddifyRust.require(hiddifyRust.generateIdentity(), 'generateIdentity');
    return GrpcClientIdentity(certificatePem: native.certificatePem, privateKeyPem: native.privateKeyPem);
  }
}
