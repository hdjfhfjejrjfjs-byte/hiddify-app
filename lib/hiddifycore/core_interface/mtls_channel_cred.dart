import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:basic_utils/basic_utils.dart';
import 'package:grpc/grpc.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';

/// mTLS credentials for the local hiddify-core gRPC server.
///
/// Trusts only [serverCertificatePem] (pinned), presents a self-signed client
/// certificate, and sets authority to `127.0.0.1` to match the server SAN.
class MTLSChannelCredentials extends ChannelCredentials {
  MTLSChannelCredentials._({
    required SecurityContext context,
    required this.clientCertificatePem,
    required String serverCertificatePem,
  }) : _ctx = context,
       super.secure(
         authority: '127.0.0.1',
         onBadCertificate: (X509Certificate cert, String host) => hiddifyRust.require(
           hiddifyRust.verifyPinnedCert(cert.der, cert.pem, serverCertificatePem, host),
           'verifyPinnedCert',
         ),
       );

  final SecurityContext _ctx;

  /// PEM of the client certificate that must be registered with the Go core
  /// via `add_grpc_client_public_key` before connecting.
  final String clientCertificatePem;

  factory MTLSChannelCredentials({
    required Uint8List serverCertificatePem,
    required AsymmetricKeyPair<PublicKey, PrivateKey> clientKey,
  }) {
    final String privateKeyPem;
    final String clientCertPem;
    if (clientKey.privateKey is RSAPrivateKey && clientKey.publicKey is RSAPublicKey) {
      final priv = clientKey.privateKey as RSAPrivateKey;
      final pub = clientKey.publicKey as RSAPublicKey;
      privateKeyPem = CryptoUtils.encodeRSAPrivateKeyToPem(priv);
      final csr = X509Utils.generateRsaCsrPem({'CN': 'HiddifyClient'}, priv, pub);
      clientCertPem = X509Utils.generateSelfSignedCertificate(
        priv,
        csr,
        3650,
        extKeyUsage: [ExtendedKeyUsage.CLIENT_AUTH],
      );
    } else if (clientKey.privateKey is ECPrivateKey && clientKey.publicKey is ECPublicKey) {
      final priv = clientKey.privateKey as ECPrivateKey;
      final pub = clientKey.publicKey as ECPublicKey;
      privateKeyPem = CryptoUtils.encodeEcPrivateKeyToPem(priv);
      final csr = X509Utils.generateEccCsrPem({'CN': 'HiddifyClient'}, priv, pub);
      clientCertPem = X509Utils.generateSelfSignedCertificate(
        priv,
        csr,
        3650,
        extKeyUsage: [ExtendedKeyUsage.CLIENT_AUTH],
      );
    } else {
      throw ArgumentError('Unsupported private key type.');
    }
    return MTLSChannelCredentials.fromPem(
      serverCertificatePem: serverCertificatePem,
      clientCertificatePem: clientCertPem,
      clientPrivateKeyPem: privateKeyPem,
    );
  }

  /// Credentials for a client identity that already exists (see GrpcClientIdentity).
  factory MTLSChannelCredentials.fromPem({
    required Uint8List serverCertificatePem,
    required String clientCertificatePem,
    required String clientPrivateKeyPem,
  }) {
    final serverPem = utf8.decode(serverCertificatePem);
    if (!serverPem.contains('-----BEGIN CERTIFICATE-----')) {
      throw ArgumentError('Server certificate must be PEM.');
    }
    if (!clientCertificatePem.contains('-----BEGIN CERTIFICATE-----')) {
      throw ArgumentError('Client certificate must be PEM.');
    }

    final ctx = SecurityContext();
    ctx.setAlpnProtocols(supportedAlpnProtocols, false);
    // PEM works across Android/desktop; DER triggers BAD_PKCS12_DATA on macOS desktop.
    ctx.setTrustedCertificatesBytes(utf8.encode(serverPem));
    ctx.useCertificateChainBytes(utf8.encode(clientCertificatePem));
    ctx.usePrivateKeyBytes(utf8.encode(clientPrivateKeyPem));

    return MTLSChannelCredentials._(
      context: ctx,
      clientCertificatePem: clientCertificatePem,
      serverCertificatePem: serverPem,
    );
  }

  @override
  SecurityContext get securityContext => _ctx;

  @override
  String? get authority => '127.0.0.1';

  @override
  bool get isSecure => true;
}
