import 'dart:io';

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

/// parse profile subscription url and headers for data
///
/// ***name parser hierarchy:***
/// - UserOverride.name
/// - `profile-title` header
/// - `content-disposition` header
/// - url fragment (remote only, example: `https://example.com/config#user`) -> name=`user`
/// - url filename extension (remote only, example: `https://example.com/config.json`) -> name=`config`
/// - if none of these methods return a non-blank string, switch(profileType)
/// - remote:  fallback to `Remote Profile`
/// - local: keep the existing name, otherwise extract it from content by protocol()
///
/// Note: the url-based steps (fragment, filename) apply to remote profiles only,
/// since local profiles have no url. Every step treats a whitespace-only result
/// as blank, so it falls through to the next step.

/// Downloads a URL in the core with the user agent given; [cancel] completing stops it.
typedef CoreFetch = Future<FetchUrlResponse> Function(String url, String userAgent, {required Future<void> cancel});

/// Told that a download attempt failed with [failure] and attempt [next] of [of] begins.
typedef DownloadRetryCallback = void Function(ProfileDownloadFailure failure, int next, int of);

/// The longest update interval a subscription can ask for, in hours: a year.
const maxUpdateIntervalHours = 24 * 365;

class ProfileParser with InfraLogger {
  // Synthetic sentinel assigned to `total` for "unlimited" traffic (subscription-userinfo total=0 or
  // missing). It MUST stay above the 10 TB "unlimited" threshold the UI uses to decide whether to show
  // "∞" (isInfinitSize() in lib/utils/number_formatters.dart, and profile_tile.dart). The previous
  // value (~857 GiB) was below that gate, so total=0 rendered as a finite cap / "quota exceeded".
  // See https://github.com/hiddify/hiddify-app/issues/1974 . 1000 TiB.
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

  ProfileParser({required DioHttpClient httpClient, CoreFetch? coreFetch})
    : _httpClient = httpClient,
      _coreFetch = coreFetch;
  TaskEither<ProfileFailure, ProfileEntriesCompanion> addLocal({
    required String id,
    required String content,
    required String tempFilePath,
    required UserOverride? userOverride,
  }) {
    return TaskEither.fromEither(populateHeaders(content: content)).flatMap(
      (populatedHeaders) => _expandLinksInFile(tempFilePath, null).flatMap(
        (_) => TaskEither.fromEither(
          parse(
            tempFilePath: tempFilePath,
            profile: ProfileEntity.local(
              id: id,
              active: true,
              name: '',
              lastUpdate: DateTime.now(),
              userOverride: userOverride,
              populatedHeaders: populatedHeaders,
            ),
          ).flatMap((profEntity) => Either.tryCatch(() => profEntity.toInsertEntry(), ProfileFailure.unexpected)),
        ),
      ),
    );
  }

  TaskEither<ProfileFailure, ProfileEntriesCompanion> addRemote({
    required String id,
    required String url,
    required String tempFilePath,
    required UserOverride? userOverride,
    CancelToken? cancelToken,
    DownloadRetryCallback? onRetry,
  }) => _downloadProfile(url, tempFilePath, cancelToken, onRetry).flatMap(
    (remoteHeaders) =>
        TaskEither.fromEither(
          populateHeaders(content: File(tempFilePath).readAsStringSync(), remoteHeaders: remoteHeaders),
        ).flatMap(
          (populatedHeaders) => _expandLinksInFile(tempFilePath, cancelToken).flatMap(
            (_) => TaskEither.fromEither(
              parse(
                tempFilePath: tempFilePath,
                profile: ProfileEntity.remote(
                  id: id,
                  active: true,
                  name: '',
                  url: url,
                  lastUpdate: DateTime.now(),
                  userOverride: userOverride,
                  populatedHeaders: populatedHeaders,
                ),
              ).flatMap((profEntity) => Either.tryCatch(() => profEntity.toInsertEntry(), ProfileFailure.unexpected)),
            ),
          ),
        ),
  );

  TaskEither<ProfileFailure, ProfileEntriesCompanion> updateRemote({
    required RemoteProfileEntity rp,
    required String tempFilePath,
    CancelToken? cancelToken,
    DownloadRetryCallback? onRetry,
  }) => _downloadProfile(rp.url, tempFilePath, cancelToken, onRetry).flatMap(
    (remoteHeaders) =>
        TaskEither.fromEither(
          populateHeaders(content: File(tempFilePath).readAsStringSync(), remoteHeaders: remoteHeaders),
        ).flatMap(
          (populatedHeaders) => _expandLinksInFile(tempFilePath, cancelToken).flatMap(
            (_) => TaskEither.fromEither(
              parse(
                tempFilePath: tempFilePath,
                profile: rp.copyWith(populatedHeaders: populatedHeaders),
              ).flatMap((profEntity) => Either.tryCatch(() => profEntity.toUpdateEntry(), ProfileFailure.unexpected)),
            ),
          ),
        ),
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

  /// The attempts at a download whose failure may pass, such as a timeout; one that will not, such as a
  /// 404, is not tried again.
  static const downloadAttempts = 2;
  static const _retryDelay = Duration(seconds: 1);

  /// Downloads the subscription, trying again once after a failure that may pass; [onRetry] is told
  /// before the next attempt, for the user to see why it takes longer.
  TaskEither<ProfileFailure, Map<String, dynamic>> _downloadProfile(
    String url,
    String tempFilePath,
    CancelToken? cancelToken,
    DownloadRetryCallback? onRetry,
  ) => TaskEither.tryCatch(() async {
    final token = cancelToken ?? CancelToken();
    for (var attempt = 1; ; attempt++) {
      try {
        return await _downloadOnce(url, tempFilePath, token);
      } on ProfileDownloadFailure catch (failure) {
        if (attempt >= downloadAttempts || !failure.problem.mayPass) rethrow;
        onRetry?.call(failure, attempt + 1, downloadAttempts);
        await Future.any([Future<void>.delayed(_retryDelay), token.whenCancel]);
        if (token.isCancelled) throw _cancelled;
      }
    }
  }, (err, st) => err is ProfileFailure ? err : ProfileFailure.unexpected(err, st));

  static const _cancelled = ProfileFailure.cancelByUser('HTTP request for getting profile content canceled by user.');

  /// One attempt: downloads the subscription in the core first, with the TLS handshake of Chrome, which some
  /// networks let through when they drop Dart's; when that fails, downloads it here, once, without
  /// retries, so a dead link fails in seconds. When both fail, the failure says why in the words of
  /// the one that tells the most (an answer from the server before a network problem), with what
  /// each one met as its detail.
  Future<Map<String, dynamic>> _downloadOnce(String url, String tempFilePath, CancelToken token) async {
    final link = url.trim();
    final problems = <(DownloadProblem, int?)>[];
    final details = <String>[];

    if (_coreFetch case final coreFetch?) {
      try {
        final rs = await coreFetch(link, _httpClient.userAgent, cancel: token.whenCancel.then((_) {}));
        if (DownloadProblem.ofCore(rs) case final problem?) {
          final hasStatus = rs.error == FetchUrlError.FETCH_URL_ERROR_NONE;
          problems.add((problem, hasStatus ? rs.statusCode : null));
          details.add('core: ${hasStatus ? 'HTTP ${rs.statusCode}' : rs.errorDetail}');
        } else {
          await File(tempFilePath).writeAsBytes(rs.body, flush: true);
          return _flattenHeaders({
            for (final header in rs.headers)
              header.name: [
                for (final h in rs.headers)
                  if (h.name == header.name) h.value,
              ],
          });
        }
      } catch (err) {
        if (token.isCancelled) throw _cancelled;
        // the core could not be asked: this says nothing about the server
        details.add('core: unavailable');
        loggy.debug('core download of the profile failed: $err');
      }
      loggy.debug('core download of the profile failed, downloading it here: ${details.last}');
    }

    try {
      final rs = await _httpClient.download(link, tempFilePath, cancelToken: token, retry: false);
      return _flattenHeaders(rs.headers.map);
    } on DioException catch (err) {
      if (CancelToken.isCancel(err)) throw _cancelled;
      problems.add((DownloadProblem.ofDio(err), err.response?.statusCode));
      details.add('app: ${DownloadProblem.detailOfDio(err)}');
    }
    final problem = DownloadProblem.mostTelling(problems.map((p) => p.$1));
    final statusCode = problems.firstWhere((p) => p.$1 == problem).$2;
    loggy.warning('profile download failed: ${problem.name}; ${details.join('; ')}');
    throw ProfileFailure.download(problem, statusCode: statusCode, detail: details.join('\n'));
  }

  /// A header with one value as a string, with more as a list.
  static Map<String, dynamic> _flattenHeaders(Map<String, List<String>> headers) =>
      hiddifyRust.require(hiddifyRust.flattenHttpHeaders(headers), 'flattenHttpHeaders');

  /// Replaces every subscription link line in [text] with the content it points to.
  ///
  /// Returns [text] unchanged when no line is a link.
  Future<String> expandLinks(String text, CancelToken cancelToken) async {
    final plan = hiddifyRust.require(
      hiddifyRust.planLinkExpansion(text, decodeBase64First: false),
      'planLinkExpansion',
    );
    if (plan.targets.isEmpty) return text;

    final replacements = <(int, String)>[];
    int targetCursor = 0;

    Future<void> worker() async {
      while (true) {
        if (cancelToken.isCancelled) return;

        final pos = targetCursor++;
        if (pos >= plan.targets.length) return;

        final target = plan.targets[pos];
        try {
          final response = await _httpClient.get<String>(
            target.url,
            cancelToken: cancelToken,
            responseType: ResponseType.plain,
          );
          replacements.add((target.index, response.data ?? ''));
        } catch (err) {
          if (err is DioException && CancelToken.isCancel(err)) return;
          // the full url carries the user's token, so only the host is logged
          loggy.warning('dropped an inner subscription link that failed to download: ${Uri.tryParse(target.url)?.host}');
          replacements.add((target.index, ''));
        }
      }
    }

    // 4 downloads at a time
    await Future.wait(List.generate(4, (_) => worker()));
    if (cancelToken.isCancelled) {
      throw const ProfileFailure.cancelByUser('HTTP request for getting an inner subscription link canceled by user.');
    }
    return hiddifyRust.require(hiddifyRust.mergeExpandedLinks(plan.decodedText, replacements), 'mergeExpandedLinks');
  }

  /// Expands the links in the temp file and writes the file once, only if the text changed.
  TaskEither<ProfileFailure, Unit> _expandLinksInFile(String tempFilePath, CancelToken? cancelToken) =>
      TaskEither.tryCatch(() async {
        final file = File(tempFilePath);
        final text = safeDecodeBase64((await file.readAsString()).trim());
        final expanded = await expandLinks(text, cancelToken ?? CancelToken());
        if (expanded != text) await file.writeAsString(expanded);
        return unit;
      }, (err, st) => err is ProfileFailure ? err : ProfileFailure.unexpected(err, st));

  static Either<ProfileFailure, Map<String, dynamic>> populateHeaders({
    required String content,
    Map<String, dynamic>? remoteHeaders,
  }) => Either.tryCatch(
    () => hiddifyRust.require(hiddifyRust.populateHeaders(content, remoteHeaders), 'populateHeaders'),
    ProfileFailure.unexpected,
  );

  /// Parse a `subscription-userinfo` header, e.g.
  /// `upload=0; download=1024; total=10240; expire=1704054600`.
  ///
  /// Tolerant of real-world noise: a segment without a `key=value` pair (a
  /// stray `;;`, a trailing `;`, or a malformed token) is skipped instead of
  /// throwing and failing the whole profile parse. Values may be decimals and
  /// are truncated to int. `upload` and `download` are required; a missing,
  /// `0` or unreadable `total`/`expire` is treated as "unlimited", and so is an
  /// `expire` past [infiniteTimeThreshold].
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

  /// Extract a filename from a `content-disposition` header.
  ///
  /// Prefers the RFC 5987 extended form `filename*=UTF-8''<percent-encoded>`,
  /// which is how servers send non-ASCII names (Persian, Chinese, emoji …),
  /// and falls back to the plain quoted form `filename="name.txt"`. Returns ''
  /// when neither is present or the extended value can't be decoded.
  @visibleForTesting
  static String filenameFromContentDisposition(String header) =>
      hiddifyRust.require(hiddifyRust.filenameFromContentDisposition(header), 'filenameFromContentDisposition');

  /// Decode the value of a `profile-title: base64:<value>` header.
  ///
  /// Accepts the value without `=` padding, which some servers send. Returns ''
  /// when it is not valid base64 or not UTF-8, so the name falls through to the
  /// next source instead of failing the whole profile parse.
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

        // Standalone headers: valid with or without `subscription-userinfo`.
        String? webPageUrl;
        if (headers['profile-web-page-url'] case final String profileWebPageUrl when isUrl(profileWebPageUrl)) {
          webPageUrl = profileWebPageUrl;
        }
        String? supportUrl;
        if (headers['support-url'] case final String profileSupportUrl when isUrl(profileSupportUrl)) {
          supportUrl = profileSupportUrl;
        }
        // `ipinfo-url` stays in populatedHeaders only (not a dedicated column).

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

  /// Resolve an `extra-security` value to a chain mode.
  ///
  /// Strict on purpose: the value must equal a documented mode key exactly, or
  /// it is ignored. `profile` is not accepted — chaining to another profile is a
  /// user choice, not something a subscription may impose.
  @visibleForTesting
  static ChainMode? extraSecurityMode(String? value) {
    if (value == null) return null;
    hiddifyRust.ensureAvailable('extraSecurityMode');
    final key = hiddifyRust.extraSecurityMode(value);
    if (key == null) return null;
    return ChainMode.values.where((m) => m.key == key).firstOrNull;
  }

  /// How many proxies run, the fastest, when neither the user nor the subscription says.
  static const keepTopDefault = 5;

  /// A `keep-top` header: how many proxies to keep running, or -1 for all. Anything else is no
  /// header at all.
  static int? keepTopHeader(String? value) {
    if (value == null) return null;
    hiddifyRust.ensureAvailable('keepTopHeader');
    return hiddifyRust.keepTopHeader(value);
  }

  /// Build the `tls-tricks` map from a subscription `fragment` header.
  ///
  /// Mirrors the per-proxy `fragment` parameter, which needs a full triplet —
  /// `size,sleep,method` or `tlshello,size,sleep`. Anything shorter, or a size
  /// or sleep that isn't a valid range, is incomplete and turns fragmentation
  /// off entirely rather than enabling it with half the values. Returns null
  /// when the header can't be used.
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
