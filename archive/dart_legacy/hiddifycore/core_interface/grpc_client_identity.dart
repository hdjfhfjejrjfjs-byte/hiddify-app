// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/hiddifycore/core_interface/grpc_client_identity.dart
// Replaced by:   rust/crates/hiddify-crypto (generate_identity, load_or_create_identity, is_identity_usable)

import 'dart:io';
import 'dart:isolate';

import 'package:basic_utils/basic_utils.dart';

class LegacyGrpcClientIdentity {
  const LegacyGrpcClientIdentity({required this.certificatePem, required this.privateKeyPem});

  final String certificatePem;
  final String privateKeyPem;

  static const certificateFile = 'client.crt';
  static const privateKeyFile = 'client.key';

  static const _validityDays = 3650;
  static const _renewBefore = Duration(days: 30);

  static Future<LegacyGrpcClientIdentity> loadOrCreate(Directory directory) async {
    final certificate = File('${directory.path}/$certificateFile');
    final privateKey = File('${directory.path}/$privateKeyFile');
    try {
      if (await certificate.exists() && await privateKey.exists()) {
        final identity = LegacyGrpcClientIdentity(
          certificatePem: await certificate.readAsString(),
          privateKeyPem: await privateKey.readAsString(),
        );
        if (identity._isUsable()) return identity;
      }
    } on FileSystemException {
      // unreadable: made again below
    }
    final identity = await Isolate.run(LegacyGrpcClientIdentity.generate);
    await directory.create(recursive: true);
    await _write(privateKey, identity.privateKeyPem);
    await _write(certificate, identity.certificatePem);
    return identity;
  }

  factory LegacyGrpcClientIdentity.generate() {
    final pair = CryptoUtils.generateRSAKeyPair();
    final privateKey = pair.privateKey as RSAPrivateKey;
    final publicKey = pair.publicKey as RSAPublicKey;
    final csr = X509Utils.generateRsaCsrPem({'CN': 'HiddifyClient'}, privateKey, publicKey);
    final certificate = X509Utils.generateSelfSignedCertificate(
      privateKey,
      csr,
      _validityDays,
      extKeyUsage: [ExtendedKeyUsage.CLIENT_AUTH],
    );
    if (!certificate.contains('-----BEGIN CERTIFICATE-----')) {
      throw StateError('Failed to generate client certificate PEM.');
    }
    return LegacyGrpcClientIdentity(
      certificatePem: certificate,
      privateKeyPem: CryptoUtils.encodeRSAPrivateKeyToPem(privateKey),
    );
  }

  bool _isUsable() {
    if (!certificatePem.contains('-----BEGIN CERTIFICATE-----') ||
        !privateKeyPem.contains('-----BEGIN PRIVATE KEY-----')) {
      return false;
    }
    try {
      final notAfter = X509Utils.x509CertificateFromPem(certificatePem).tbsCertificate?.validity.notAfter;
      return notAfter != null && notAfter.isAfter(DateTime.now().add(_renewBefore));
    } on Object {
      return false;
    }
  }

  static Future<void> _write(File file, String content) async {
    final temporary = File('${file.path}.tmp');
    await temporary.writeAsString(content, flush: true);
    await temporary.rename(file.path);
  }
}
