import 'dart:async';
import 'dart:convert';
import 'dart:isolate';

import 'package:dio/dio.dart';
import 'package:fpdart/fpdart.dart';
import 'package:hiddify/core/db/db.dart';
import 'package:hiddify/core/http_client/dio_http_client.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/features/profile/data/profile_data_mapper.dart';
import 'package:hiddify/features/profile/model/download_problem.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';
import 'package:hiddify/features/profile/model/profile_failure.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hiddify/singbox/model/singbox_config_enum.dart';
import 'package:hiddify/utils/utils.dart';
import 'package:meta/meta.dart';

/// Test-only hook signature for injecting a synthetic `CoreFetch` response in unit tests.
typedef CoreFetch = Future<FetchUrlResponse> Function(String url, String userAgent, {required Future<void> cancel});

/// Told that a download attempt failed with [failure] and attempt [next] of [of] begins.
typedef DownloadRetryCallback = void Function(ProfileDownloadFailure failure, int next, int of);

/// The longest update interval a subscription can ask for, in hours: a year.
const maxUpdateIntervalHours = 24 * 365;

class ProfileParser with InfraLogger {
  static const infiniteTrafficThreshold = 1_099_511_627_776_000;
  static const infiniteTimeThreshold = 92_233_720_368;
  static const allowedOverrideConfigs = [
    'ipv6-mode',
    'connection-test-url',
    'ipinfo-url',
    'direct-dns-address',
    'remote-dns-address',
    'tls-tricks',
    'chain-status',
    'extra-security',
    'keep-top',
    'keep-top-quick',
    'keep-top-countries',
    'keep-top-pinned',
    'keep-tags',
  ];
  static const allowedProfileHeaders = [
    'profile-title',
    'content-disposition',
    'subscription-userinfo',
    'profile-update-interval',
    'support-url',
    'profile-web-page-url',
    'ipinfo-url',
    'extra-security',
    'fragment',
    'keep-top',
  ];

  final DioHttpClient _httpClient;
  final CoreFetch? _coreFetch;
  final bool _useRustCoreDirect;

  /// Production constructor: executes Chrome-TLS direct download, proxy/direct fallback,
  /// 4-way concurrent inner-link expansion, header extraction, and single-pass file write in Rust.
  ProfileParser.native({required DioHttpClient httpClient})
    : _httpClient = httpClient,
      _coreFetch = null,
      _useRustCoreDirect = true;

  /// Test-compatible constructor supporting optional [coreFetch] mock injection.
  ProfileParser({required DioHttpClient httpClient, CoreFetch? coreFetch})
    : _httpClient = httpClient,
      _coreFetch = coreFetch,
      _useRustCoreDirect = false;

  TaskEither<ProfileFailure, ProfileEntriesCompanion> addLocal({
    required String id,
    required String content,
    required String tempFilePath,
    required UserOverride? userOverride,
  }) => TaskEither.tryCatch(() async {
    final userAgent = _httpClient.userAgent;
    final proxyPort = _httpClient.port;
    final proxySecret = _httpClient.secret;
    final meta = _metadataInput(userOverride: userOverride);
    final result = await Isolate.run(
      () => hiddifyRust.require(
        hiddifyRust.prepareLocalProfile({
          'content': content,
          'tempFilePath': tempFilePath,
          'userAgent': userAgent,
          'proxyPort': proxyPort,
          'proxySecret': proxySecret,
          'timeoutMs': 15000,
          'cancelTaskId': 0,
          'metadata': meta,
        }),
        'prepareLocalProfile',
      ),
    );
    final resolved = _extractResolvedOrThrow(result);
    return ProfileEntity.local(
      id: id,
      active: true,
      name: resolved.name,
      lastUpdate: DateTime.now(),
      userOverride: userOverride,
      populatedHeaders: resolved.populatedHeaders,
    ).toInsertEntry();
  }, (err, st) => err is ProfileFailure ? err : ProfileFailure.unexpected(err, st));

  TaskEither<ProfileFailure, ProfileEntriesCompanion> addRemote({
    required String id,
    required String url,
    required String tempFilePath,
    required UserOverride? userOverride,
    CancelToken? cancelToken,
    DownloadRetryCallback? onRetry,
  }) => _runRemotePipeline(
    url: url,
    tempFilePath: tempFilePath,
    userOverride: userOverride,
    cancelToken: cancelToken,
    onRetry: onRetry,
  ).flatMap(
    (resolved) => Either.tryCatch(
      () => ProfileEntity.remote(
        id: id,
        active: true,
        name: resolved.name,
        url: url,
        lastUpdate: DateTime.now(),
        options: resolved.options,
        subInfo: resolved.subInfo,
        webPageUrl: resolved.webPageUrl,
        supportUrl: resolved.supportUrl,
        userOverride: userOverride,
        populatedHeaders: resolved.populatedHeaders,
      ).toInsertEntry(),
      ProfileFailure.unexpected,
    ).toTaskEither(),
  );

  TaskEither<ProfileFailure, ProfileEntriesCompanion> updateRemote({
    required RemoteProfileEntity rp,
    required String tempFilePath,
    CancelToken? cancelToken,
    DownloadRetryCallback? onRetry,
  }) => _runRemotePipeline(
    url: rp.url,
    tempFilePath: tempFilePath,
    userOverride: rp.userOverride,
    cancelToken: cancelToken,
    onRetry: onRetry,
  ).flatMap(
    (resolved) => Either.tryCatch(
      () => rp
          .copyWith(
            name: resolved.name,
            lastUpdate: DateTime.now(),
            options: resolved.options,
            subInfo: resolved.subInfo,
            webPageUrl: resolved.webPageUrl,
            supportUrl: resolved.supportUrl,
            populatedHeaders: resolved.populatedHeaders,
          )
          .toUpdateEntry(),
      ProfileFailure.unexpected,
    ).toTaskEither(),
  );

  Either<ProfileFailure, ProfileEntriesCompanion> offlineUpdate({
    required ProfileEntity profile,
    required String tempFilePath,
  }) => profile
      .map(
        remote: (rp) => parse(profile: rp, tempFilePath: tempFilePath),
        local: (lp) => parse(tempFilePath: tempFilePath, profile: lp),
      )
      .flatMap((profEntity) => Either.tryCatch(() => profEntity.toUpdateEntry(), ProfileFailure.unexpected));

  static const downloadAttempts = 2;
  static const _retryDelay = Duration(seconds: 1);
  static const _cancelled = ProfileFailure.cancelByUser('HTTP request for getting profile content canceled by user.');
  static const _innerCancelled = ProfileFailure.cancelByUser(
    'HTTP request for getting an inner subscription link canceled by user.',
  );

  TaskEither<ProfileFailure, _ResolvedMeta> _runRemotePipeline({
    required String url,
    required String tempFilePath,
    required UserOverride? userOverride,
    CancelToken? cancelToken,
    DownloadRetryCallback? onRetry,
  }) => TaskEither.tryCatch(() async {
    final token = cancelToken ?? CancelToken();
    for (var attempt = 1; ; attempt++) {
      try {
        return await _runRemoteAttemptOnce(
          url: url,
          tempFilePath: tempFilePath,
          userOverride: userOverride,
          token: token,
        );
      } on ProfileDownloadFailure catch (failure) {
        if (attempt >= downloadAttempts || !failure.problem.mayPass) rethrow;
        onRetry?.call(failure, attempt + 1, downloadAttempts);
        await Future.any([Future<void>.delayed(_retryDelay), token.whenCancel]);
        if (token.isCancelled) throw _cancelled;
      }
    }
  }, (err, st) => err is ProfileFailure ? err : ProfileFailure.unexpected(err, st));

  Future<_ResolvedMeta> _runRemoteAttemptOnce({
    required String url,
    required String tempFilePath,
    required UserOverride? userOverride,
    required CancelToken token,
  }) async {
    if (token.isCancelled) throw _cancelled;
    final link = url.trim();
    final userAgent = _httpClient.userAgent;
    final proxyPort = _httpClient.port;
    final proxySecret = _httpClient.secret;
    final meta = _metadataInput(userOverride: userOverride, remoteUrl: url);

    final taskId = hiddifyRust.require(hiddifyRust.createTask(), 'createTask');
    unawaited(token.whenCancel.then((_) => hiddifyRust.cancelTask(taskId)));

    try {
      Map<String, dynamic>? priorCoreFailure;

      // Support unit-test mock `CoreFetch` closures (`profile_parser_download_test.dart`).
      if (_coreFetch case final coreFetch?) {
        try {
          final rs = await coreFetch(link, userAgent, cancel: token.whenCancel.then((_) {}));
          if (token.isCancelled) throw _cancelled;
          if (DownloadProblem.ofCore(rs) case final problem?) {
            final hasStatus = rs.error == FetchUrlError.FETCH_URL_ERROR_NONE;
            priorCoreFailure = {
              'problemIndex': problem.index,
              'statusCode': hasStatus ? rs.statusCode : null,
              'detail': 'core: ${hasStatus ? 'HTTP ${rs.statusCode}' : rs.errorDetail}',
            };
          } else {
            final bodyB64 = base64.encode(rs.body);
            final hdrs = [
              for (final h in rs.headers) {'name': h.name, 'value': h.value},
            ];
            final finalized = await Isolate.run(
              () => hiddifyRust.require(
                hiddifyRust.finalizeDownloadedRemote({
                  'bodyB64': bodyB64,
                  'headers': hdrs,
                  'tempFilePath': tempFilePath,
                  'userAgent': userAgent,
                  'proxyPort': proxyPort,
                  'proxySecret': proxySecret,
                  'timeoutMs': 15000,
                  'cancelTaskId': taskId,
                  'metadata': meta,
                }),
                'finalizeDownloadedRemote',
              ),
            );
            if (token.isCancelled) throw _cancelled;
            return _extractResolvedOrThrow(finalized);
          }
        } catch (err) {
          if (token.isCancelled || err is ProfileCancelByUserFailure) throw _cancelled;
          priorCoreFailure = {
            'problemIndex': null,
            'statusCode': null,
            'detail': 'core: unavailable',
          };
          loggy.debug('core download of the profile failed: $err');
        }
      }

      final skipCoreDirect = !_useRustCoreDirect || priorCoreFailure != null;
      final res = await Isolate.run(
        () => hiddifyRust.require(
          hiddifyRust.fetchAndPrepareRemoteAttempt({
            'url': link,
            'tempFilePath': tempFilePath,
            'userAgent': userAgent,
            'proxyPort': proxyPort,
            'proxySecret': proxySecret,
            'coreTimeoutMs': 30000,
            'appTimeoutMs': 15000,
            'skipCoreDirect': skipCoreDirect,
            if (priorCoreFailure != null) 'priorCoreFailure': priorCoreFailure,
            'cancelTaskId': taskId,
            'metadata': meta,
          }),
          'fetchAndPrepareRemoteAttempt',
        ),
      );
      if (token.isCancelled) throw _cancelled;
      return _extractResolvedOrThrow(res);
    } finally {
      hiddifyRust.removeTask(taskId);
    }
  }

  static Map<String, dynamic> _metadataInput({
    UserOverride? userOverride,
    String? remoteUrl,
    String? existingLocalName,
  }) => {
    if (userOverride?.name != null) 'overrideName': userOverride!.name,
    if (userOverride?.updateInterval != null) 'overrideUpdateInterval': userOverride!.updateInterval,
    'isAutoUpdateDisable': userOverride?.isAutoUpdateDisable ?? false,
    if (remoteUrl != null) 'remoteUrl': remoteUrl,
    if (existingLocalName != null) 'existingLocalName': existingLocalName,
  };

  _ResolvedMeta _extractResolvedOrThrow(Map<String, dynamic> res) {
    final status = res['status'] as String?;
    switch (status) {
      case 'ok':
        final r = res['resolved'] as Map<String, dynamic>;
        final hours = (r['updateIntervalHours'] as num?)?.toInt();
        SubscriptionInfo? subInfo;
        if (r['subInfo'] case final Map<String, dynamic> sub) {
          subInfo = SubscriptionInfo(
            upload: (sub['upload'] as num).toInt(),
            download: (sub['download'] as num).toInt(),
            total: (sub['total'] as num).toInt(),
            expire: DateTime.fromMillisecondsSinceEpoch(((sub['expire_secs'] ?? sub['expireSecs']) as num).toInt() * 1000),
          );
        }
        return _ResolvedMeta(
          name: r['name'] as String,
          options: hours == null ? null : ProfileOptions(updateInterval: Duration(hours: hours)),
          subInfo: subInfo,
          webPageUrl: r['webPageUrl'] as String?,
          supportUrl: r['supportUrl'] as String?,
          populatedHeaders: Map<String, dynamic>.from((r['populatedHeaders'] as Map?) ?? const {}),
        );
      case 'cancelled':
        throw _cancelled;
      case 'download_failed':
        final problemIdx = (res['problemIndex'] as num).toInt();
        final problem = DownloadProblem.values[problemIdx.clamp(0, DownloadProblem.values.length - 1)];
        final statusCode = (res['statusCode'] as num?)?.toInt();
        final detail = res['detail'] as String?;
        loggy.warning('profile download failed: ${problem.name}; ${detail?.replaceAll('\n', '; ') ?? ''}');
        throw ProfileFailure.download(problem, statusCode: statusCode, detail: detail);
      default:
        throw StateError((res['error'] as String?) ?? 'profile pipeline failed');
    }
  }

  /// Replaces every subscription link line in [text] with the content it points to via native Rust.
  Future<String> expandLinks(String text, CancelToken cancelToken) async {
    if (cancelToken.isCancelled) throw _innerCancelled;
    final userAgent = _httpClient.userAgent;
    final proxyPort = _httpClient.port;
    final proxySecret = _httpClient.secret;
    final taskId = hiddifyRust.require(hiddifyRust.createTask(), 'createTask');
    unawaited(cancelToken.whenCancel.then((_) => hiddifyRust.cancelTask(taskId)));
    try {
      final res = await Isolate.run(
        () => hiddifyRust.require(
          hiddifyRust.expandLinksNative(
            text: text,
            decodeBase64First: false,
            userAgent: userAgent,
            proxyPort: proxyPort,
            proxySecret: proxySecret,
            timeoutMs: 15000,
            cancelTaskId: taskId,
          ),
          'expandLinksNative',
        ),
      );
      if (cancelToken.isCancelled || res.status == 'cancelled') throw _innerCancelled;
      if (res.status != 'ok' || res.text == null) {
        throw StateError(res.error ?? 'expandLinksNative failed');
      }
      return res.text!;
    } finally {
      hiddifyRust.removeTask(taskId);
    }
  }

  static Either<ProfileFailure, Map<String, dynamic>> populateHeaders({
    required String content,
    Map<String, dynamic>? remoteHeaders,
  }) => Either.tryCatch(
    () => hiddifyRust.require(hiddifyRust.populateHeaders(content, remoteHeaders), 'populateHeaders'),
    ProfileFailure.unexpected,
  );

  static SubscriptionInfo? _parseSubscriptionInfo(String subInfoStr) {
    hiddifyRust.ensureAvailable('parseSubscriptionInfo');
    final parsed = hiddifyRust.parseSubscriptionInfo(subInfoStr);
    if (parsed == null) return null;
    return SubscriptionInfo(
      upload: parsed.upload,
      download: parsed.download,
      total: parsed.total,
      expire: DateTime.fromMillisecondsSinceEpoch(parsed.expireSecs * 1000),
    );
  }

  @visibleForTesting
  static String filenameFromContentDisposition(String header) =>
      hiddifyRust.require(hiddifyRust.filenameFromContentDisposition(header), 'filenameFromContentDisposition');

  @visibleForTesting
  static String decodeBase64Title(String value) =>
      hiddifyRust.require(hiddifyRust.decodeBase64Title(value), 'decodeBase64Title');

  @visibleForTesting
  static Either<ProfileFailure, ProfileEntity> parse({required String tempFilePath, required ProfileEntity profile}) =>
      Either.tryCatch(() {
        final headers = Map<String, dynamic>.from(profile.populatedHeaders ?? {});
        final name = hiddifyRust.require(
          hiddifyRust.resolveProfileName(
            overrideName: profile.userOverride?.name,
            profileTitleHeader: headers['profile-title'] is String ? headers['profile-title'] as String : null,
            contentDispositionHeader: headers['content-disposition'] is String
                ? headers['content-disposition'] as String
                : null,
            remoteUrl: profile is RemoteProfileEntity ? profile.url : null,
            existingLocalName: profile is LocalProfileEntity ? profile.name : null,
            tempFilePath: tempFilePath,
          ),
          'resolveProfileName',
        );

        hiddifyRust.ensureAvailable('parseUpdateIntervalHours');
        final intervalHours = hiddifyRust.parseUpdateIntervalHours(
          headerValue: headers['profile-update-interval'] is String
              ? headers['profile-update-interval'] as String
              : null,
          overrideHours: profile.userOverride?.updateInterval,
          isAutoUpdateDisable: profile.userOverride?.isAutoUpdateDisable ?? false,
        );
        final options = intervalHours == null ? null : ProfileOptions(updateInterval: Duration(hours: intervalHours));

        SubscriptionInfo? subInfo;
        if (headers['subscription-userinfo'] case final String subInfoStr) {
          subInfo = _parseSubscriptionInfo(subInfoStr);
        }

        String? webPageUrl;
        if (headers['profile-web-page-url'] case final String profileWebPageUrl when isUrl(profileWebPageUrl)) {
          webPageUrl = profileWebPageUrl;
        }
        String? supportUrl;
        if (headers['support-url'] case final String profileSupportUrl when isUrl(profileSupportUrl)) {
          supportUrl = profileSupportUrl;
        }

        return profile.map(
          remote: (rp) => rp.copyWith(
            name: name,
            lastUpdate: DateTime.now(),
            options: options,
            subInfo: subInfo,
            webPageUrl: webPageUrl,
            supportUrl: supportUrl,
          ),
          local: (lp) => lp.copyWith(name: name, lastUpdate: DateTime.now()),
        );
      }, ProfileFailure.unexpected);

  static String protocol(String content) => hiddifyRust.require(hiddifyRust.detectProtocol(content), 'detectProtocol');

  static String profileOverrideHelper({required ProfileEntriesCompanion profile}) {
    return ProfileParser.profileOverride(
      populatedHeaders: decodeProfileHeaders(profile.populatedHeaders.value),
      userOverride: decodeUserOverride(profile.userOverride.value),
    );
  }

  @visibleForTesting
  static ChainMode? extraSecurityMode(String? value) {
    if (value == null) return null;
    hiddifyRust.ensureAvailable('extraSecurityMode');
    final key = hiddifyRust.extraSecurityMode(value);
    if (key == null) return null;
    return ChainMode.values.where((m) => m.key == key).firstOrNull;
  }

  static const keepTopDefault = 5;

  static int? keepTopHeader(String? value) {
    if (value == null) return null;
    hiddifyRust.ensureAvailable('keepTopHeader');
    return hiddifyRust.keepTopHeader(value);
  }

  @visibleForTesting
  static Map<String, dynamic>? fragmentTricks(String? value) {
    if (value == null) return null;
    hiddifyRust.ensureAvailable('fragmentTricks');
    return hiddifyRust.fragmentTricks(value);
  }

  static String profileOverride({
    required Map<String, dynamic>? populatedHeaders,
    required UserOverride? userOverride,
  }) {
    final uoMap = userOverride == null
        ? null
        : <String, dynamic>{
            if (userOverride.enableFragment != null) 'enableFragment': userOverride.enableFragment,
            if (userOverride.extraSecurity != null) 'extraSecurity': userOverride.extraSecurity,
            if (userOverride.keepTop != null) 'keepTop': userOverride.keepTop,
            if (userOverride.keepTopQuick != null) 'keepTopQuick': userOverride.keepTopQuick,
            if (userOverride.keepTopCountries != null) 'keepTopCountries': userOverride.keepTopCountries,
            if (userOverride.keepTopPinned != null) 'keepTopPinned': userOverride.keepTopPinned,
            if (userOverride.keepTags != null) 'keepTags': userOverride.keepTags,
          };
    return hiddifyRust.require(hiddifyRust.profileOverride(populatedHeaders, uoMap), 'profileOverride');
  }

  static Map<String, dynamic> applyProfileOverride(Map<String, dynamic> main, String? profileOverride) {
    if (profileOverride == null || !profileOverride.contains("{")) return main;
    return hiddifyRust.require(hiddifyRust.applyProfileOverride(main, profileOverride), 'applyProfileOverride');
  }
}

class _ResolvedMeta {
  const _ResolvedMeta({
    required this.name,
    required this.options,
    required this.subInfo,
    required this.webPageUrl,
    required this.supportUrl,
    required this.populatedHeaders,
  });

  final String name;
  final ProfileOptions? options;
  final SubscriptionInfo? subInfo;
  final String? webPageUrl;
  final String? supportUrl;
  final Map<String, dynamic> populatedHeaders;
}
