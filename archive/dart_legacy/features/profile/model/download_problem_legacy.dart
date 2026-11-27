// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/features/profile/model/download_problem.dart
// Replaced by:   rust/crates/hiddify-profile (download::DownloadProblemKind)

import 'dart:io';

import 'package:dio/dio.dart';
import 'package:hiddify/features/profile/model/download_problem.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';

abstract class LegacyDownloadProblem {
  static bool mayPass(DownloadProblem p) => switch (p) {
    DownloadProblem.notFound ||
    DownloadProblem.denied ||
    DownloadProblem.badStatus ||
    DownloadProblem.tooLarge ||
    DownloadProblem.certificate => false,
    _ => true,
  };

  static DownloadProblem ofStatus(int code) => switch (code) {
    >= 500 => DownloadProblem.serverError,
    404 || 410 => DownloadProblem.notFound,
    401 || 403 => DownloadProblem.denied,
    429 => DownloadProblem.tooManyRequests,
    _ => DownloadProblem.badStatus,
  };

  static DownloadProblem? ofCore(FetchUrlResponse response) => switch (response.error) {
    FetchUrlError.FETCH_URL_ERROR_NONE =>
      response.statusCode >= 200 && response.statusCode <= 299 ? null : ofStatus(response.statusCode),
    FetchUrlError.FETCH_URL_ERROR_TIMEOUT => DownloadProblem.timeout,
    FetchUrlError.FETCH_URL_ERROR_DNS => DownloadProblem.dns,
    FetchUrlError.FETCH_URL_ERROR_REFUSED => DownloadProblem.refused,
    FetchUrlError.FETCH_URL_ERROR_RESET => DownloadProblem.reset,
    FetchUrlError.FETCH_URL_ERROR_UNREACHABLE => DownloadProblem.noInternet,
    FetchUrlError.FETCH_URL_ERROR_TLS => DownloadProblem.tls,
    FetchUrlError.FETCH_URL_ERROR_CERTIFICATE => DownloadProblem.certificate,
    FetchUrlError.FETCH_URL_ERROR_TOO_LARGE => DownloadProblem.tooLarge,
    _ => DownloadProblem.unknown,
  };

  static DownloadProblem ofDio(DioException err) {
    if (err.response?.statusCode case final code? when code < 200 || code > 299) return ofStatus(code);
    switch (err.type) {
      case DioExceptionType.connectionTimeout || DioExceptionType.sendTimeout || DioExceptionType.receiveTimeout:
        return DownloadProblem.timeout;
      case DioExceptionType.badCertificate:
        return DownloadProblem.certificate;
      default:
    }
    final cause = err.error;
    if (cause is CertificateException) return DownloadProblem.certificate;
    final text = '${cause ?? ''} ${err.message ?? ''}'.toLowerCase();
    if (text.contains('certificate')) return DownloadProblem.certificate;
    if (text.contains('failed host lookup') || text.contains('nodename nor servname') || text.contains('no address')) {
      return DownloadProblem.dns;
    }
    if (text.contains('network is unreachable') || text.contains('no route to host')) {
      return DownloadProblem.noInternet;
    }
    if (text.contains('refused')) return DownloadProblem.refused;
    if (text.contains('reset by peer') ||
        text.contains('connection reset') ||
        text.contains('forcibly closed') ||
        text.contains('terminated during handshake') ||
        text.contains('broken pipe')) {
      return DownloadProblem.reset;
    }
    if (text.contains('timed out')) return DownloadProblem.timeout;
    if (cause is HandshakeException || cause is TlsException) return DownloadProblem.tls;
    return DownloadProblem.unknown;
  }

  static DownloadProblem mostTelling(Iterable<DownloadProblem> problems) =>
      problems.fold(DownloadProblem.unknown, (a, b) => b.index < a.index ? b : a);
}
